# 05. Analysis Specification

Analysis turns TIR (what the code says) into facts (what the code means for memory). Translation consumes the facts to choose Rust forms. This document defines every analysis, its inputs and outputs, the algorithms it should use, and what to do when it cannot decide.

Status: the baseline algorithms are **Specified**. Refinements marked _(refinement)_ are **Outlined**.

---

## 1. Principles

### 1.1 Analyses produce hypotheses, not truths

Every fact is a claim about all executions of the program. Static analysis cannot be perfect, and the original program may rely on behavior the analysis cannot see (for example an external caller). Therefore every fact has a **confidence level** and a **justification**, and verification (document 07) is the final authority.

### 1.2 Soundness stance

- **Soundness direction.** Analyses MUST err toward the safe side for the consequence that matters: when unsure, assume more aliasing, more nullability, less ownership knowledge. A wrong "may alias" only costs performance (a runtime-checked form is chosen). A wrong "no alias" could produce incorrect or unsafe output. Therefore **"no alias" claims need proof; "may alias" is the default.**
- **Closed world vs open world.** In _whole-program mode_ (the program's `main` and all callers are present) analyses may assume callers they can see are the only callers. In _library mode_ (public API visible to unknown callers) every exported function's parameters are treated as coming from an unknown caller unless a contract states otherwise (see section 12).

### 1.3 Confidence levels

```
Proven   - follows from analysis rules with no unverified assumption
Likely   - follows from analysis plus a heuristic or an observed-only pattern (for example every call site passes non-null)
Assumed  - taken from a user contract or a library model
Unknown  - analysis could not decide
```

Translation rules specify the minimum confidence each strategy requires. Zero-cost strategies (T0) require `Proven` or `Assumed`-by-contract. `Likely` facts MAY drive T1/T2 choices and are always listed in the report as assumptions.

### 1.4 Dependency order

```
macro/config resolution (frontend)
   -> type & layout facts
   -> intraprocedural CFG + liveness + reaching definitions
   -> allocation-site registration
   -> points-to (alias) analysis
   -> escape & thread-sharing
   -> nullability
   -> array extent
   -> ownership inference
   -> mutability & borrow-conflict check
   -> lifetime/region inference
   -> UB detection (uses nearly all of the above)
   -> tier hints
```

UB detection and ownership inference feed each other lightly (for example a proven use-after-free invalidates an ownership hypothesis); implementations SHOULD run ownership once, run UB detection, and re-run ownership only for functions where UB findings changed an assumption.

### 1.5 Output summary

`AnalysisSummary { facts_computed, unknown_counts_by_kind, findings, solver_calls, solver_unknowns, wall_time_per_analysis }`

---

## 2. Points-to (alias) analysis

### 2.1 Goal

For every pointer-typed place (and every reference), compute the set of abstract memory objects it may point to. This determines which pointers may alias, which in turn determines whether `&mut` and `&` can coexist.

### 2.2 Abstract objects

```
AbstractObject =
  | Stack(LocalId, instance)         // address-taken local
  | Global(ItemId)
  | Heap(AllocSiteId, ctx)           // allocation site, optionally with a call-site context
  | Field(AbstractObject, FieldIdx)  // for field-sensitivity
  | StringLit(id)
  | Param(FnId, idx)                 // incoming object for an exported function in library mode
  | Unknown                          // escaped or external memory
  | Null
```

### 2.3 Algorithm (baseline) [Decision]

An **inclusion-based (Andersen-style) analysis**, field-sensitive, flow-insensitive with a flow-sensitive refinement for locals, context-insensitive by default, optionally 1-call-site-sensitive for functions flagged as allocation wrappers.

Constraint forms (where `pts(x)` is the points-to set of `x`):

```
p = &x          =>  { x } ⊆ pts(p)
p = q           =>  pts(q) ⊆ pts(p)
p = *q          =>  for each o in pts(q): pts(o) ⊆ pts(p)
*p = q          =>  for each o in pts(p): pts(q) ⊆ pts(o)
p = &q->f       =>  for each o in pts(q): { Field(o,f) } ⊆ pts(p)
p = malloc(...) =>  { Heap(site) } ⊆ pts(p)
p = q + k       =>  pts(q) ⊆ pts(p)                 // pointer arithmetic stays inside the object (array-insensitive)
call f(a)       =>  bind actuals to formals; bind return value to the call result; resolve indirect/virtual targets on the fly
```

Solving: worklist propagation with **cycle collapsing** (online cycle detection) and **difference propagation** to reach near-linear behavior in practice. Solve per strongly connected component of the call graph in bottom-up order; use function summaries (section 11) to avoid re-solving callees for each caller.

### 2.4 Field sensitivity

Track one points-to set per field for records. For arrays, collapse all elements into one abstract element. For unions, collapse all members into one object. Nested access `a.b.c` is expanded into projections.

### 2.5 Handling difficult constructs

| Construct                                          | Treatment                                                                                                                                                                          |
| -------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Pointer arithmetic                                 | Result points to the same abstract object(s) as the operand; element index is tracked separately by array-extent analysis                                                          |
| Integer to pointer casts, pointer to integer casts | Result points to `Unknown`; the cast is a UB/portability finding (`UB-009` class) unless it is a recognized idiom (alignment masking, `uintptr_t` round trip in a single function) |
| `memcpy`/`memmove` between pointers                | Treated as copying pointee contents: for each pair of objects `(src_o, dst_o)`, `pts(fields of src_o) ⊆ pts(fields of dst_o)`                                                      |
| `realloc`                                          | New heap object that inherits contents; old object considered freed (UB if used after)                                                                                             |
| Function pointers                                  | Resolved by points-to; unresolved targets become edges to `Unknown`                                                                                                                |
| Virtual calls                                      | Receivers resolved by class hierarchy intersected with receiver points-to                                                                                                          |
| `setjmp`/`longjmp`                                 | Whole function is flagged; analysis treats it as opaque                                                                                                                            |
| External functions with no model                   | Arguments that are pointers escape to `Unknown`; return value may point to `Unknown`                                                                                               |
| Threads                                            | Objects reachable from thread-start arguments are marked `thread_shared`                                                                                                           |

### 2.6 Outputs

`points_to: Place -> Set<AbstractObject>` and a derived query API:

- `may_alias(p, q) -> bool` (conservative)
- `must_not_alias(p, q) -> Justified<bool>` (requires disjoint points-to sets, or proof via flow-sensitive reasoning that they point to different live objects at the relevant point)
- `points_to_unique_owner_candidate(p) -> Option<AbstractObject>`

### 2.7 Complexity and scale

Baseline Andersen is cubic in the worst case but near-linear on real code with the optimizations above. The implementation MUST support a **fallback to a unification-based (Steensgaard-style) analysis** for items or whole programs where the inclusion solver exceeds a configured constraint budget; the fallback is less precise (more aliasing) and therefore only costs performance.

---

## 3. Ownership inference

### 3.1 Goal

For every pointer-typed _place_ (local, parameter, return value, field of a record, global), decide what role it plays in memory management. This is the analysis that determines whether a C++ `T*` becomes `Box<T>`, `&T`, `&mut T`, `Option<...>`, an index, or something runtime-checked.

**[Research]** The Crown system (Zhang, David, Yu, Wang; CAV 2023) shows that a constraint-based, field- and flow-sensitive ownership analysis can scale to half a million lines in under ten seconds, and that it handles nested pointers and inductive data structures (linked lists, trees). It does so by making a strengthening assumption about the Rust ownership model that removes the need for a separate aliasing analysis. **[Research]** Its documented limitations (array pointers lacking metadata for synthesis, mutable local borrows needing lifetime analysis, access paths that break its ownership monotonicity assumption) are exactly the gaps recast fills with its array-extent analysis (section 5), lifetime analysis (section 8) and tiered fallbacks (document 06).

recast's ownership inference is inspired by this approach but is not a copy. The design below is the contract; the builder MAY implement it differently if the outputs and guarantees hold.

### 3.2 Ownership domain

```
OwnershipKind =
  | Owning                      // this pointer is responsible for freeing its pointee
  | BorrowedShared              // read-only view of something owned elsewhere
  | BorrowedMut                 // read-write view; exclusive for its live range
  | SharedOwner(RcCandidate)    // multiple owners (manual refcount, shared_ptr)
  | Unknown                     // analysis cannot classify; forces a conservative tier
Extent  = Single | Array(ExtentInfo)       // from section 5
Nullable = bool                            // from section 4
```

Pointee-level structure: for pointers to records containing pointers, the analysis assigns an `OwnershipKind` to **each pointer field path** (for example `Node.next`), not just to the top-level pointer. Two pointers of the same C++ type can have different roles in different places.

### 3.3 Variables and constraints [Decision]

For each pointer-typed place `x` at each relevant program point, a boolean variable `own(x)` (true = owning). Constraints are generated from statements. Representative rules (`R` = rule ID):

| ID      | Statement                                                              | Constraint                                                                                                                                                                      |
| ------- | ---------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| OWN-001 | `x = malloc(..)` / `new T` / `make_unique<T>()`                        | `own(x) = true` (fresh allocation, `nonnull` unless result is checked)                                                                                                          |
| OWN-002 | `free(x)` / `delete x`                                                 | `own(x) = true` before the call; after, `x` is dead (no owner)                                                                                                                  |
| OWN-003 | `y = x` (pointer copy)                                                 | If `own(x)` before and `own(y)` after, then `own(x)` after is false (ownership moves). If `own(y)` is false, `x`'s ownership is unchanged (y is a borrow)                       |
| OWN-004 | read through `x` only; never freed, never stored in an owning slot     | `own(x) = false`, kind = `BorrowedShared`                                                                                                                                       |
| OWN-005 | write through `x`                                                      | kind = `BorrowedMut` or `Owning` (owner may write)                                                                                                                              |
| OWN-006 | `return x`                                                             | `own(result) = own(x)` before return (ownership flows to caller) unless `x` is a borrow of a parameter, in which case result is a borrow with a lifetime tied to that parameter |
| OWN-007 | `f(x)` with callee summary parameter ownership `P`                     | If `P = Consumes`, `own(x)` before and none after; if `P = Borrows`, no change                                                                                                  |
| OWN-008 | store `s->f = x`                                                       | If field `f` is `Owning` role, `own(x)` before and none after; if `f` is a borrow role, no ownership change                                                                     |
| OWN-009 | `x = NULL` after use                                                   | Supports flow-sensitivity: `x` is dead as an owner after `free`; assignment of null makes it a null non-owner                                                                   |
| OWN-010 | two pointers freed to the same allocation on some path                 | Conflict: double free finding `UB-004`; ownership of that allocation is `Unknown`                                                                                               |
| OWN-011 | pointer stored in a global or a data structure reachable from a thread | Mark `thread_shared`; ownership role must be a shared owner or `Unknown` unless a single owner can be shown                                                                     |

**Ownership monotonicity assumption (stated for transparency).** For tractability the analysis assumes that, along any access path, an owned object is owned by at most one pointer at a time and that transfer is explicit. This is deliberately stricter than Rust requires in some cases (**[Research]** Crown notes this strictness). Where the assumption fails, the constraints become unsatisfiable and the offending pointers fall to `Unknown` (see 3.5) rather than producing wrong output.

### 3.4 Solving

Encode as a boolean satisfiability problem and solve with the solver trait (`ConstraintSolver`). Use incremental solving per function and per record-field path, with function summaries linking calls. A satisfying assignment gives the ownership roles; the solver MUST be run with fixed seeds and logical-step limits for determinism.

### 3.5 When constraints are unsatisfiable or timeouts occur

1. Extract a minimal unsatisfiable core (or an approximation).
2. Mark the pointers in the core as `Unknown`; record a finding with the core (location list) so a human can see which statements conflict.
3. Remove those pointers' constraints; re-solve the rest.
4. Pointers marked `Unknown` MUST translate at tier T2 or higher (document 06 section 3).

### 3.6 Recognized allocation patterns

The following are registered as allocation/deallocation pairs and fed into the constraints:

- C: `malloc`/`calloc`/`realloc`/`free`, `strdup`, `fopen`/`fclose`, `opendir`/`closedir` (resource handles)
- C++: `new`/`delete`, `new[]`/`delete[]`, placement new (flagged), `std::make_unique`, `std::make_shared`, `std::allocator` direct use (flagged)
- Project-specific wrappers (for example `xmalloc`, `my_free`): detected as _allocation wrappers_ when a function returns a freshly allocated object with no other reference retained, and listed in the contracts file by the user if the detector misses them

### 3.7 Shared ownership

Manual reference counting (a counter field incremented and decremented around pointer copies) and `std::shared_ptr` are classified `SharedOwner`. Translation maps these to `Rc`/`Arc` (document 06). Detection of manual refcounting is _(refinement)_: baseline treats such pointers as `Unknown` unless the user marks the type in the contracts file.

---

## 4. Nullability

### 4.1 Goal

Decide, for every pointer-typed place at every program point, whether it may be null.

### 4.2 Algorithm

A forward dataflow analysis over the CFG with a three-point lattice per place: `{NonNull, MaybeNull, Null}` joined as `NonNull ⊔ Null = MaybeNull`.

Refinements from guards:

- `if (p)`, `if (p != NULL)`, `if (!p) return;`, `assert(p)`, early-exit patterns refine `p` on the corresponding branch.
- Assignment from an allocation: `MaybeNull` (allocation can fail) _unless_ the allocator is `new` (throws; non-null unless `nothrow`) or a configured assumption `assume_malloc_succeeds` is on, in which case the result is `NonNull` with confidence `Assumed`.
- Dereference without prior guard: evidence the original programmer assumed `NonNull`; contributes `Likely NonNull` to parameters when _all_ observed call sites pass non-null values (call-site evidence).
- Parameters of exported functions in library mode: `MaybeNull` unless contracted.

### 4.3 Outputs and consequences

`nullability: Place -> {NonNull, MaybeNull}` with confidence.

- `NonNull` and `Proven/Assumed` -> reference or `Box` without `Option`.
- `MaybeNull` -> `Option<...>`.
- Dereference of a `MaybeNull` pointer without guard is a finding (`UB-006`) and the translation inserts an explicit check that panics (see document 06 section 11).

---

## 5. Array extent and pointer arithmetic

### 5.1 Goal

Determine whether a pointer points to a single object or into an array, and where the length comes from. This is what allows `(ptr, len)` pairs to become `&[T]` and pointer-walking loops to become iterators.

### 5.2 Extent domain

```
Extent =
  | Single
  | Fixed(n)                      // array of known compile-time length
  | Len(Place)                    // length is held in another variable or field (for example parameter n)
  | NulTerminated                 // C string convention
  | Sentinel(const)               // terminated by a specific value
  | Unknown
```

### 5.3 Evidence rules

| ID      | Evidence                                                                                                                          | Extent inferred                                                              |
| ------- | --------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------- |
| ARR-001 | Pointer comes from a fixed-size array decay                                                                                       | `Fixed(n)`                                                                   |
| ARR-002 | Pointer from `malloc(k * sizeof(T))` with count expression `k`                                                                    | `Len(k)` or `Fixed(k)` when constant                                         |
| ARR-003 | Every index `p[i]` is dominated by a check `i < n` (or the loop condition implies it) and `n` is a parameter or field next to `p` | `Len(n)`                                                                     |
| ARR-004 | `p` is passed to `strlen`, `strcpy`, `printf("%s")`                                                                               | `NulTerminated`                                                              |
| ARR-005 | Struct has pointer field followed (or preceded) by an integer field used as bound in every access                                 | `Len(field)`                                                                 |
| ARR-006 | Two-pointer range `[begin, end)` pairs used in loops                                                                              | `Len(end - begin)` (translated to slice or iterator)                         |
| ARR-007 | Flexible array member at the end of a struct                                                                                      | `Len(...)` from the struct's length field if one is found; otherwise flagged |
| ARR-008 | `std::vector` data, `std::array`, `std::span`                                                                                     | `Len(self.size())` / `Fixed(N)`                                              |

### 5.4 Relational bounds analysis

To justify ARR-003, track relations such as `i < n`, `i <= n - 1`, `i + k < n`. Use an abstract domain combining intervals with symbolic upper bounds (a simplified relational domain such as "difference-bound" constraints `x - y <= c`). Loop induction variables get widening with narrowing from the loop condition.

### 5.5 Pointer arithmetic classification

| Pattern                                                      | Classification                                       | Translation hint                           |
| ------------------------------------------------------------ | ---------------------------------------------------- | ------------------------------------------ |
| `for (i = 0; i < n; i++) use(p[i])`                          | in-bounds indexed traversal                          | slice indexing or iterator                 |
| `for (q = p; q != p + n; ++q) use(*q)`                       | pointer-walk traversal                               | iterator                                   |
| `p + k` passed as a sub-buffer with remaining length `n - k` | sub-slice                                            | `&p[k..]`                                  |
| `q - p`                                                      | index difference when both point into the same array | `q_idx - p_idx`                            |
| `p++` consuming a stream with end check                      | cursor over slice                                    | index plus slice or `iter()` with `next()` |
| Pointer comparison across unrelated objects                  | UB (`UB-020`)                                        | flag                                       |
| Arithmetic that can leave `[p, p+n]`                         | UB (`UB-019`)                                        | flag or runtime check                      |

Pointer walks that cannot be mapped to indices fall back to **T3** (contained `unsafe` with explicit bound assertions) or T4. The Cpp2Rust measurements (**[Research]** about 6x slowdown on a pointer-arithmetic-heavy benchmark when using runtime-enforced safety) justify keeping this classification precise: it is the main lever for performance.

### 5.6 Outputs

`array_extent: Place -> Extent`, with the justification chain (which checks dominated which accesses), and a `bounds_proved: bool` per access site used later by translation to decide whether to emit plain indexing (Rust's indexing is always bounds-checked, so a proof means _the check cannot fail_, not that it is removed) or an explicit guard.

---

## 6. Build configurations and conditional compilation

C/C++ code is not one program; it is a family of programs selected by preprocessor conditions. Translators that preprocess first (running the C preprocessor and translating the result) discard this structure.

**[Research]** The Hayroll system (PLDI 2026) identifies two problems with preprocess-first translators: they are incomplete on configurable software and they lose programmer-defined abstractions embedded in macros. Its remedy is a wrapper with two layers: a conditional-compilation layer that uses symbolic execution to derive activation conditions for each source line and splits the program into configuration-specific translation tasks, and a macro layer that classifies macros, tags macro-expanded nodes, and reconstructs macros as Rust functions or macros from the tags that survive translation. It reports preserving correctness across configurations and avoiding configuration explosion through symbolic execution.

recast adopts that architecture as the model for its own frontend (section 6.1) and macro handling (section 7).

### 6.1 Configuration model

```
ConfigurationDescriptor {
  id: ConfigId,
  atoms: Map<AtomName, Value>,       // macro defines and platform atoms
  source: FromCompileCommand | UserSupplied | Derived,
}
ConfigCondition = boolean formula over atoms
```

Modes (set in `recast.toml`, see document 09):

- `single` (default): one configuration, taken from the compile command. Fast, covers one build.
- `listed`: user lists configurations (for example `linux-x86_64-release`, `windows-debug`).
- `derived`: recast derives the set of conditions under which each line is active by symbolically executing the preprocessor, then emits Rust with `#[cfg(...)]` attributes for conditions that depend on **feature-like** atoms and chooses a representative configuration for **platform-like** atoms.

### 6.2 Symbolic activation conditions

Walk each file's preprocessor directives, treating undefined macros as symbolic. Each `#if`/`#ifdef`/`#elif`/`#else` produces a condition conjunct; nested conditions combine by conjunction; the line's activation condition is the conjunction of enclosing conditions. Evaluate constant sub-expressions concretely when possible. Simplify with a boolean solver; cap the number of distinct atoms considered (configurable) to avoid combinatorial explosion, falling back to `single` mode for the capped file with a finding.

### 6.3 Atom classification

| Class          | Examples                                  | Rust mapping                                                                    |
| -------------- | ----------------------------------------- | ------------------------------------------------------------------------------- |
| Feature atoms  | `USE_FAST`, `ENABLE_LOGGING`, `HAVE_ZLIB` | Cargo feature; `#[cfg(feature = "use_fast")]`                                   |
| Platform atoms | `__linux__`, `_WIN32`, `__x86_64__`       | `#[cfg(target_os = ...)]`, `#[cfg(target_arch = ...)]`                          |
| Value atoms    | `VERSION >= 3`, `BUFSIZE`                 | const generics or `const` items per configuration; or fixed at translation time |
| Compiler atoms | `__GNUC__`, `_MSC_VER`                    | Fixed to the configured toolchain; the other branch is dropped with a note      |
| Guard atoms    | include guards (`FOO_H`)                  | Removed; modules replace them                                                   |

### 6.4 Output

Items and statements carry `ConfigCondition` in provenance. Translation emits `#[cfg(...)]` where the condition is expressible, and otherwise emits the code for the chosen configuration plus a report entry listing the dropped alternatives.

---

## 7. Macro analysis

### 7.1 Macro taxonomy [Decision, informed by Hayroll]

The frontend classifies every macro definition and every use.

| Class           | Description                                                                                                                                    | Example                               | Target                                                                                                                  |
| --------------- | ---------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------- | ----------------------------------------------------------------------------------------------------------------------- |
| ObjectConstant  | Constant value                                                                                                                                 | `#define MAX 100`                     | `const MAX: i32 = 100;`                                                                                                 |
| ObjectExpr      | Constant expression                                                                                                                            | `#define MASK (1u<<5)`                | `const`                                                                                                                 |
| FnExprPure      | Function-like macro whose body is an expression using each argument at most once, no side effects on args, no dependence on surrounding locals | `#define SQ(x) ((x)*(x))`             | `#[inline] fn` (when argument types are uniform at all uses) or generic `fn`                                            |
| FnExprMultiEval | Evaluates an argument more than once                                                                                                           | `#define MAX(a,b) ((a)>(b)?(a):(b))`  | `macro_rules!` preserving evaluation, or `fn` when arguments are side-effect-free at all call sites                     |
| FnStmt          | Statement-like macro, `do { } while(0)` wrapper                                                                                                | `#define LOG(...) do {...} while(0)`  | `macro_rules!`                                                                                                          |
| ControlFlow     | Contains `return`, `goto`, `break`, `continue` that escape into the caller                                                                     | `#define CHECK(x) if(!(x)) return -1` | `macro_rules!` (cannot be a function)                                                                                   |
| DeclGenerating  | Declares variables, functions or types                                                                                                         | `#define DECL(T) T T##_var;`          | Expand (or `macro_rules!` when output items are static)                                                                 |
| TypeMacro       | Expands to a type                                                                                                                              | `#define PTR(T) T*`                   | type alias or generic alias                                                                                             |
| Stringify/Paste | Uses `#` or `##`                                                                                                                               | `#define NAME(x) x##_t`               | `macro_rules!` with `stringify!`/`paste` (the `paste` crate is acceptable only behind a recorded decision), else expand |
| XMacro          | List macro applied with different definitions                                                                                                  | `#define LIST(X) X(a) X(b)`           | Expand per use; optionally `macro_rules!`                                                                               |
| VariadicMacro   | Uses `__VA_ARGS__`                                                                                                                             | `#define LOG(fmt, ...)`               | `macro_rules!` with repetition                                                                                          |
| FeatureToggle   | Defined/undefined to select code                                                                                                               | `#define USE_FAST`                    | Cargo feature                                                                                                           |
| IncludeGuard    | `#ifndef X_H` pattern                                                                                                                          | n/a                                   | Removed                                                                                                                 |

### 7.2 Algorithm

1. The exporter reports macro definitions, each expansion event (name, definition, argument token spans, expansion span) and which AST nodes originate from which expansion (macro trail).
2. The frontend classifies each definition using syntactic and, where needed, semantic checks (argument use counts from the token body; presence of control-flow tokens; type-dependence by checking whether the argument spellings used at all call sites share the same type).
3. For classes with a function or macro target, the frontend builds the macro as a TIR `MacroDef` and rewrites each expansion site into a call, **provided** a _round-trip check_ passes: expanding the generated Rust form must correspond to the original expansion's AST (same operator structure) for every call site. Sites that fail the round-trip are expanded in place and recorded.
4. Anything not meeting these conditions is expanded (the always-correct fallback).

### 7.3 Why not always expand

Expanding preserves behavior but destroys abstractions: the output has the macro body copied at every use. The report records the percentage of macro uses that were preserved as abstractions versus expanded.

### 7.4 Hygiene hazards to check

- Argument evaluated multiple times with side effects.
- Operator precedence dependence (missing parentheses).
- Variable capture (macro body references names from the use site).
- Name shadowing created by statement macros.
  Any hazard flagged at a use site forces expansion at that site.

---

## 8. Mutability, borrow conflicts and lifetime inference

### 8.1 Mutability

`mutability: Place -> ReadOnly | ReadWrite` is derived from stores through the pointer and from `const` qualification. A `const_cast` that removes const, or `mutable` members, are noted (`mutable` maps to `Cell`/`RefCell`).

### 8.2 Borrow-conflict check

For each function, using liveness and points-to, determine whether any program point has two live references to overlapping memory where at least one is mutable. If none: `&`/`&mut` is directly usable (T0). If some: the conflicting references are marked `NeedsInteriorMutability` (T2) or `NeedsIndexing` (T1) depending on the data (section 8.4 of document 06).

### 8.3 Lifetime and region inference

A reference may only be used while its referent lives. Rust expresses this with lifetimes; many are inferred by the compiler (elision rules), others need explicit annotations.

Algorithm:

1. Assign a region variable to each reference-typed place.
2. Generate outlives constraints from assignments (`r2 = r1` implies `r1: r2`), returns (`return r` implies `r: 'fn_result`), field stores, and calls (using callee signature regions).
3. Solve the constraint system for the least regions (non-lexical, flow-based, similar in spirit to Rust's NLL).
4. Derive function signatures: if the result borrows from exactly one parameter, elision applies (`fn f(x: &T) -> &U`); if from several, emit explicit lifetime parameters; if from none but a reference is returned, the reference must be `'static` or the construct is wrong (finding).
5. Struct fields that hold references need lifetime parameters on the struct; the analysis lists them. When the structure of lifetimes becomes too complex (more than a configured number of distinct region parameters on a type, or recursive borrows), the translation prefers owned data, `Rc`, or handles over lifetime-parametrized types.

_(refinement)_ **Compiler as oracle.** **[Research]** Laertes (Emre et al.) infers lifetimes and ownership by repeatedly asking the Rust compiler whether a candidate rewrite type-checks and using its error messages to adjust. recast MAY add an optional refinement loop that attempts a more precise form, runs `cargo check` with the pinned toolchain, and falls back to the previous form on failure. This is deterministic as long as the Rust toolchain version is pinned and recorded. Each accepted refinement MUST still pass the full verification ladder.

---

## 9. Escape and thread-sharing analysis

### 9.1 Escape

For every allocation site and address-taken local, compute where its address may flow: `NoEscape`, `EscapesToReturn`, `EscapesToHeap`, `EscapesToThread`, `EscapesUnknown`. Uses:

- `NoEscape` heap allocations can become stack values or `Vec`/array locals (a heap allocation that does not escape can be a local).
- `EscapesToReturn` determines the ownership flowing to callers.
- `EscapesUnknown` forces conservative handling.

### 9.2 Thread sharing

An object is `thread_shared` if reachable from arguments passed to `std::thread`, `pthread_create`, `std::async`, or from globals accessed by functions reachable from such roots. Consequences:

- Shared ownership uses `Arc`, not `Rc`.
- Mutation of shared objects uses `Mutex`/`RwLock`/atomics (matching the original synchronization primitives when identified), not `RefCell`/`Cell`.
- Shared state with no recognized synchronization is a possible data race (`UB-011`) and a finding.

---

## 10. Undefined-behavior (UB) detection

### 10.1 Purpose

C++ programs often contain operations whose behavior the standard does not define. A faithful translation must decide what to do with them, and a trustworthy tool must tell the user. The UB analysis finds sites where the original behavior depends on UB, and records how the translation treats them.

### 10.2 Catalogue

Each finding has a stable ID, a detection method, and a default translation behavior.

| ID     | UB category                                                              | Static detection                                       | Dynamic detection (sanitizer-assisted) | Default Rust behavior                                                                  | Policy default   |
| ------ | ------------------------------------------------------------------------ | ------------------------------------------------------ | -------------------------------------- | -------------------------------------------------------------------------------------- | ---------------- |
| UB-001 | Signed integer overflow                                                  | Range analysis shows overflow possible                 | UBSan `signed-integer-overflow`        | Config: `panic` (checked, default) or `wrap`                                           | Report           |
| UB-002 | Out-of-bounds array/pointer access                                       | Array-extent analysis cannot prove bounds              | ASan, UBSan `bounds`                   | Rust indexing panics                                                                   | Report           |
| UB-003 | Use after free                                                           | Ownership analysis shows access after `free` on a path | ASan                                   | Not representable in safe Rust; ownership forces correct order, else T4                | Reject if proven |
| UB-004 | Double free                                                              | Two owners freed on same path                          | ASan                                   | Same as UB-003                                                                         | Reject if proven |
| UB-005 | Read of uninitialized value                                              | Definite-assignment analysis                           | MSan                                   | Variables initialized explicitly (`0`/`Default`) at declaration, recorded              | Report           |
| UB-006 | Null pointer dereference                                                 | Nullability analysis                                   | ASan                                   | `Option` plus explicit check/panic                                                     | Report           |
| UB-007 | Invalid shift (count >= width or negative)                               | Range analysis                                         | UBSan `shift`                          | Checked shift (`checked_shl`) with panic                                               | Report           |
| UB-008 | Division/modulo by zero, `INT_MIN / -1`                                  | Range analysis                                         | UBSan                                  | Rust panics                                                                            | Report           |
| UB-009 | Strict-aliasing violation / type punning                                 | Cast and access-type mismatch                          | none practical                         | Flag; no safe general mapping; byte-copy via `from_le_bytes` etc. when layout is known | Flag             |
| UB-010 | Misaligned access                                                        | Cast analysis                                          | UBSan `alignment`                      | Use `read_unaligned`-style helper in T3 or byte copy                                   | Flag             |
| UB-011 | Data race                                                                | Thread-sharing without synchronization                 | TSan                                   | Compile-time rejection by Rust (`Send`/`Sync`); falls to Mutex/atomics tier            | Report           |
| UB-012 | Allocation/deallocation mismatch (`malloc`/`delete`, `new[]`/`delete`)   | Allocation-site matching                               | ASan                                   | Use consistent strategy; finding recorded                                              | Report           |
| UB-013 | Modifying a string literal or const object                               | Constant analysis                                      | ASan/UBSan                             | `&'static str`/`&[u8]` immutable; mutation forces a copy at translation                | Report           |
| UB-014 | Falling off the end of a non-void function                               | CFG reachability                                       | UBSan `return`                         | Explicit `unreachable!()`/panic, recorded                                              | Report           |
| UB-015 | Dangling reference or pointer use after scope end                        | Lifetime analysis                                      | ASan (use-after-scope)                 | Rust lifetimes forbid; else T2/T4                                                      | Reject if proven |
| UB-016 | Out-of-range enum value                                                  | Range analysis                                         | UBSan `enum`                           | Checked conversion `TryFrom`                                                           | Report           |
| UB-017 | Overlapping `memcpy`                                                     | Alias analysis                                         | ASan interceptors                      | `copy_within` / panic on overlap check                                                 | Report           |
| UB-018 | Infinite loop without side effects (C++ forward progress)                | Loop analysis                                          | none                                   | Preserved as written                                                                   | Note             |
| UB-019 | Pointer arithmetic outside array bounds                                  | Extent analysis                                        | UBSan `pointer-overflow`               | Index math with checks                                                                 | Report           |
| UB-020 | Comparing unrelated pointers                                             | Points-to disjointness                                 | none                                   | Flag; compare addresses via `as usize` only in T3                                      | Flag             |
| UB-021 | Unsequenced side effects (for example `i = i++ + 1`)                     | Evaluation-order analysis                              | UBSan (limited)                        | Left-to-right fixed order; recorded                                                    | Report           |
| UB-022 | Virtual call on object with ended lifetime / during construction         | Lifetime and construction analysis                     | ASan                                   | Preserved only if analysis proves safe                                                 | Report           |
| UB-023 | Violating a function's `[[noreturn]]` or `noexcept` contract at run time | Call-graph exception analysis                          | none                                   | `panic`/abort per config                                                               | Report           |

Detection methods: **static** runs in the analysis stage; **dynamic** is performed by verification when the project has runnable tests (document 07) and the results are merged into the same finding list by location.

### 10.3 Handling policy

For each finding, `ub_policy` (document 09) selects:

- `report` (default): translate to the Rust-defined behavior from the table, list the site in the report.
- `strict`: any finding with confidence `Proven` or `Likely` causes the _item_ to be flagged (T4) so the original behavior is preserved exactly (including its UB), and the user decides.
- `reject`: abort the run with an error on `Proven` findings (used in audit contexts).

Key honesty rule: replacing UB with defined behavior changes behavior **only on executions that already had UB**. The report states this and lists every such site, because customers may have shipped code whose observable behavior depends on how a particular compiler treated the UB.

### 10.4 Integer semantics decision

C++ unsigned arithmetic wraps; signed overflow is UB. Rust arithmetic panics on overflow in debug builds and wraps in release builds unless `overflow-checks` is enabled. [Decision] recast emits:

- Unsigned arithmetic: explicit wrapping operations (`wrapping_add`, `wrapping_mul`, and so on) wherever overflow is _possible_ by range analysis, so behavior matches C++ exactly regardless of build profile.
- Signed arithmetic: plain operators with `overflow-checks = true` set in the generated crate's release profile (config `int_overflow = "panic"`, default), so overflow panics instead of being silently wrapped; or explicit `wrapping_*` operations when `int_overflow = "wrap"`, which matches what typical hardware compilers do in practice but hides a latent bug.
- Casts between integer types use `as` only when the C++ conversion is value-preserving or implementation-defined wrap (narrowing of integers modulo 2^n is defined for unsigned destination and implementation-defined for signed in C++ before C++20; Rust `as` truncates, which matches the common behavior). Out-of-range float-to-int casts are UB in C++; Rust `as` saturates; flagged as `UB-016`-class.

---

## 11. Interprocedural strategy and function summaries

### 11.1 Summaries

Each function receives a summary computed bottom-up over call-graph SCCs:

```
FnSummary {
  param_ownership: Vec<Consumes | Borrows | BorrowsMut | Stores | Unknown>,
  param_nullability: Vec<Requires NonNull | MayBeNull>,
  param_extents: Vec<Extent>,
  returns: Fresh(Owning) | BorrowOf(ParamIdx) | Static | Unknown,
  may_free: Set<ParamIdx>,
  escapes: Set<ParamIdx>,
  may_throw: bool,
  side_effects: Pure | ReadsGlobals | WritesGlobals | Io | Unknown,
  thread_safety: Reentrant | NeedsLock | Unknown,
}
```

Inside an SCC, iterate to a fixed point with a deterministic order (sort by `ItemId`) and a bounded iteration count; on non-convergence, mark involved summaries `Unknown` fields.

### 11.2 Library models

Calls to external functions (libc, POSIX, the C++ standard library) use **model files** (`models/libc.toml`, `models/posix.toml`, `models/cxx-std.toml`) shipped with recast. A model states, per function: ownership of arguments and results, nullability, buffer-size relationships (for example `memcpy(dst, src, n)` writes `n` bytes into `dst`, reads `n` bytes from `src`, requires non-overlap), side effects, and whether the function can fail with `errno`.

Example (`models/libc.toml`):

```toml
[memcpy]
params = [
  { name = "dst", ownership = "borrows_mut", nonnull = true, extent = { len = "n" } },
  { name = "src", ownership = "borrows",     nonnull = true, extent = { len = "n" } },
  { name = "n" },
]
returns = { alias_of = "dst" }
requires = ["no_overlap(dst, src, n)"]
side_effects = "writes_through(dst)"

[fopen]
params = [{ name = "path", ownership = "borrows", nonnull = true, extent = "nul_terminated" },
          { name = "mode", ownership = "borrows", nonnull = true, extent = "nul_terminated" }]
returns = { ownership = "owning", nullable = true, release = "fclose" }
```

Missing models default to `unknown`: the call forces conservative handling of its pointer arguments.

### 11.3 Whole-program vs modular mode

- **Whole-program**: all callers visible; exported-function assumptions are derived from observed callers.
- **Modular** (typical for incremental migration): the module's public API is analyzed in library mode; assumptions come from contracts (section 12). Calls from the module to code outside it use contracts or models.

---

## 12. Contracts (user-supplied assumptions)

Contracts let the user state facts the analysis cannot derive, especially at API boundaries. They are read from `recast-contracts.toml` (document 09).

```toml
[[function]]
name = "parse_header"
[function.params.buf]
ownership = "borrowed"        # borrowed | owned | consumed
nullable = false
length_param = "len"          # extent Len(len)
[function.returns]
ownership = "owned"           # caller must free
nullable = true               # NULL on error

[[type]]
name = "Node"
[type.fields.next]
ownership = "owned"
nullable = true

[[allocator]]
alloc = "xmalloc"
free  = "xfree"
```

Rules:

- A contract fact has confidence `Assumed`.
- Verification MUST test contracts where possible (for example call-site checks in instrumented builds: assert non-null, assert length matches) and report any violation observed.
- A contract that conflicts with proven analysis results is a finding (`ContractViolation`) and the analysis result wins unless the user marks the contract `force = true`.

---

## 13. Template and constexpr analysis

- **Instantiation facts.** The exporter supplies each used instantiation with concrete type arguments. Analysis runs on instantiations as ordinary code.
- **Instantiation families.** Group instantiations of the same template; used by the generic-recovery refinement (document 06 section 12) to decide whether to re-express them as a Rust generic function.
- **SFINAE and tag dispatch.** If a template's behavior selection depends on SFINAE (overload resolution failure used as a feature test), the _resolved_ instantiation is still translated correctly because Clang already resolved it; but the generic form is not recoverable, so the item is marked `monomorphic_only`.
- **constexpr evaluation.** Use Clang's constant-evaluated results (the exporter exposes values of `constexpr` variables and constant expressions). `constexpr` functions translate to `const fn` when the body fits Rust's const evaluation rules; otherwise a normal function plus a precomputed constant for known arguments.

---

## 14. Tier hints

Analysis emits `tier_hint(item)`: the _lowest_ tier that the facts justify for each pointer/place in an item, and the evidence. Translation takes the maximum (most conservative) over everything in a function to set that function's tier and records, per place, the chosen strategy. Conditions:

| Facts                                                       | Lowest justified strategy                               |
| ----------------------------------------------------------- | ------------------------------------------------------- |
| `Owning`, `NonNull`, single owner, no conflicting aliases   | T0 `Box<T>` / owned value                               |
| `BorrowedShared`/`BorrowedMut`, region solved, no conflicts | T0 `&T` / `&mut T`                                      |
| Nullable versions of the above                              | T0 with `Option`                                        |
| Array extent known                                          | T0 slice or `Vec`                                       |
| Shared ownership detected                                   | T2 `Rc`/`Arc` (reference counting is the runtime check) |
| Overlapping `&mut` or shared mutation, `Copy` data          | T2 `Cell` (zero overhead)                               |
| Overlapping `&mut` or shared mutation, non-`Copy` data      | T2 `RefCell` or `Mutex`                                 |
| Cyclic or graph-like structure with no clear owner          | T1 arena + handles, else T2                             |
| Pointer arithmetic unprovable, extent unknown               | T3 (contained `unsafe`)                                 |
| `Unknown` ownership, unsupported construct                  | T3 or T4                                                |

---

## 15. Testing the analyses

Each analysis has three layers of tests (see document 10):

1. **Unit tests** on tiny IR fragments with hand-computed expected facts.
2. **Micro-benchmark suites** of C/C++ snippets, each tagged with the expected ownership/alias/nullability/extent facts (ground truth derived by manual reading).
3. **Soundness checks via dynamic validation**: instrument the original program (for example with a shadow tracker of allocation ownership and aliasing under a test suite) and verify that no execution contradicts a `Proven` fact. A contradiction is a P0 bug.

Metrics tracked per analysis: precision (share of pointers classified `Proven` rather than `Unknown`), solver time, memory, and number of fallbacks.
