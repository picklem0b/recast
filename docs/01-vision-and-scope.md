# 01. Vision and Scope

## 1. The problem

Large C and C++ codebases carry a class of defects, memory-safety bugs, that cannot be eliminated by testing alone. Buffer overflows, use-after-free, double-free, uninitialized reads and data races are consequences of how the languages let programs touch memory.

**[Research]** The scale of the problem is well documented:

- Roughly 70% of serious security vulnerabilities in large C/C++ codebases are memory-safety bugs (Microsoft and Google data cited widely; Chromium reports a similar share).
- CISA (U.S. Cybersecurity and Infrastructure Security Agency) asked software manufacturers to publish memory-safety roadmaps by 1 January 2026, and its "Secure by Design" guidance recommends memory-safe languages for new critical-infrastructure products.
- DARPA runs the TRACTOR program ("Translating All C to Rust"), whose stated goal is automated translation producing Rust of the quality and style a skilled Rust developer would write.
- GrammaTech's DARPA-supported C++ to Rust migration offering achieved "Awardable" status on the Department of Defense's Platform One marketplace in September 2025.

Rust offers memory safety with no garbage collector, so it is the natural target. But rewriting millions of lines by hand is slow, expensive and itself error-prone. Existing automated tools mostly fall into two camps:

1. **Faithful transliterators** (for example C2Rust) that preserve behavior but emit `unsafe`, C-shaped Rust. The result compiles and runs, but gives no memory-safety benefit until someone manually lifts it.
2. **Research prototypes** (ownership-guided translation, LLM-assisted pipelines, safe C++ translation) that achieve safety on parts of the problem but are not packaged as a product with verification evidence a regulated customer can audit.

**[Research]** The closest recent result for C++ is _Cpp2Rust_ (PLDI 2026), which translates C++ into functionally equivalent memory-safe Rust by inserting runtime-enforced ownership and mutability checks, reporting about 2% overhead on one benchmark and about 6x slowdown on another dominated by pointer arithmetic. That trade-off (safety bought with runtime checks) is a core design input to recast.

## 2. What recast is

recast is a **source-to-source translator plus verifier plus migration helper**:

1. **Translator.** Reads C/C++ and emits Rust. The translation is rule-based and deterministic.
2. **Verifier.** Checks every output against the original with a layered set of techniques, and records exactly what was and was not established.
3. **Migration helper.** Lets teams move one module at a time while the rest stays in C/C++, by generating and checking the boundary between the two worlds.

The product the customer receives is the combination of the Rust output and the **verification report**. In safety-critical and regulated settings the report is as important as the code, because it is the evidence that the migration did not change behavior.

## 3. What recast is not

- It is **not a new programming language.** Nobody writes recast source.
- It is **not a code assistant** that explains errors or suggests fixes interactively.
- It is **not a one-click trust button.** Every output passes through the safety gate or is rejected.
- It is **not an AI product.** The core path contains no machine-learning components (see ADR-003). An optional AI layer is a late-stage extension that can only propose, never decide.
- It is **not a general C++ compiler or analyzer.** It uses Clang for understanding code; it does not replace Clang.

## 4. Users and use cases

| User                                                               | Need                                                                                   | What recast gives them                                       |
| ------------------------------------------------------------------ | -------------------------------------------------------------------------------------- | ------------------------------------------------------------ |
| Security or platform team at a company with a large C/C++ codebase | A defensible plan to reduce memory-safety exposure and satisfy a memory-safety roadmap | Module-by-module migration with evidence                     |
| Defense and government contractors                                 | Auditable migration of legacy code to memory-safe languages                            | Deterministic output, signed verification report             |
| Embedded and safety-critical teams                                 | Safe rewrites where bugs have physical consequences                                    | Strict safety-gate profiles, no hidden `unsafe`              |
| Infrastructure library maintainers                                 | Ship a Rust version of a widely used C/C++ library                                     | Idiomatic output, FFI compatibility layer for existing users |

## 5. Goals

### 5.1 Primary goals

- **G1. Correctness first.** The Rust output behaves like the original on all inputs the verification system can examine, and every known divergence is reported.
- **G2. Real memory safety.** The output uses the least `unsafe` the analysis can justify. Every `unsafe` block is individually justified and listed in the report.
- **G3. Determinism.** The same inputs, configuration and tool version produce byte-identical output and an identical report (apart from a declared metadata block).
- **G4. Honesty.** Constructs that cannot be translated safely are flagged and, by default, preserved behind an FFI stub so the build still works. Nothing is hidden.
- **G5. Incremental migration.** Any subset of modules can be migrated while the remainder stays in C/C++, with a verified boundary between them.
- **G6. Readable output.** The Rust is formatted, organized by module, and idiomatic wherever safety and behavior allow, because humans will maintain it.

### 5.2 Secondary goals

- **G7. Scale.** Analyze codebases of several hundred thousand to millions of lines in practical time, using parallelism and caching. **[Research]** Ownership-inference research (Crown) reports analyzing half a million lines of C in under ten seconds, which shows the analysis class is capable of this scale.
- **G8. Explainability.** For any output construct, a user can ask "why was it translated this way?" and get the rule identifier, the analysis facts used, and the alternatives rejected.
- **G9. Measurability.** Report the share of code at each memory-safety tier and the measured runtime-check overhead.

## 6. Non-goals

- Translating _every_ valid C++ program. Heavy template metaprogramming, inline assembly, `setjmp`/`longjmp`, and some forms of type punning are outside automatic translation (see section 8).
- Matching hand-tuned performance on pointer-intensive code in early milestones. Runtime-checked fallbacks cost performance by design, and the report quantifies it.
- Translating build systems wholesale. recast integrates with existing builds; it does not replace them.
- Translating other languages (Objective-C, Fortran, and so on).
- Proving general equivalence of arbitrary programs. recast claims only what its verification layers actually establish (see `07-verification-specification.md`).

## 7. Principles

1. **Deterministic core.** No randomness, no network, no ML models, no wall-clock dependence in translation. Seeds, where unavoidable (fuzzing), are recorded.
2. **Verify everything; trust nothing.** Analysis results are hypotheses. The safety gate decides.
3. **Prefer the cheapest justified strategy.** Choose the zero-cost Rust form if the analysis can justify it; otherwise the next cheapest, and record why the cheaper ones were rejected.
4. **Never lie about safety.** The report counts `unsafe`, runtime checks and flagged constructs honestly. A claim of "safe" requires `#![forbid(unsafe_code)]` to hold for that crate, enforced by the compiler.
5. **Preserve behavior, then improve form.** Correct first output, then an idiom pass that is itself verified.
6. **Migrate in steps.** The unit of migration is a module or library with a defined boundary.
7. **Fail loudly and locally.** When a function cannot be translated, only that function degrades (to a flagged FFI stub); the rest of the crate still builds.
8. **Prior art is a resource, not a dependency of the core path.** Learn from C2Rust, Crown, Laertes, Hayroll and Cpp2Rust; do not make recast's correctness depend on code or models whose behavior cannot be audited.

## 8. Supported language subset

The full C++ language is too large to target at once. recast defines **language levels**. Each level is a strict superset of the previous one. Milestones in `11-roadmap-and-milestones.md` map to these levels.

### Level L0: Well-behaved C

C99/C11 (and C-style C++) with: functions, structs, enums, typedefs, global variables, fixed-size and heap-allocated arrays, `malloc`/`free`, strings via `char*`, `printf`-family calls with literal formats, standard control flow, `switch`, function pointers, and simple preprocessor use (object-like constants, include guards, simple conditional compilation).

### Level L1: Simple C++

L0 plus: classes with constructors, destructors, member functions and access control; references; namespaces; `new`/`delete`; `std::vector`, `std::string`, `std::unique_ptr`, `std::optional`, `std::array`, `std::pair`/`std::tuple`; range-based `for`; `auto`; operator overloading; `const` and `constexpr`; `enum class`; lambdas without complex captures.

### Level L2: Intermediate C++

L1 plus: single inheritance and virtual functions; abstract classes and interfaces; `std::shared_ptr`/`std::weak_ptr`; exceptions with analyzable `throw`/`catch`; function and class templates (translated per instantiation); `std::map`/`std::unordered_map`/`std::set`; `<algorithm>` calls; threads, mutexes, atomics; lambdas with captures; move semantics.

### Level L3: Advanced C++

L2 plus: multiple inheritance of interfaces; curiously recurring template patterns; SFINAE-free constrained templates and C++20 concepts (translated per instantiation); coroutines (flagged); `std::variant`/`std::any`; custom allocators; user-defined literals; heavy macro libraries.

### Always out of automatic scope (flagged and preserved behind FFI)

- Inline assembly
- `setjmp` / `longjmp`
- Computed `goto` (a GNU extension)
- Type punning through unions or pointer casts whose layout dependence cannot be shown safe
- Self-modifying code, `dlopen`-based dynamic loading logic
- Variadic functions defined by the user (calling `printf` with literal formats is supported)
- Template metaprogramming that depends on SFINAE tricks to select behavior and cannot be resolved by instantiation
- Code that relies on undefined behavior in a way the UB analysis cannot classify (see `05-analysis-specification.md`)

"Flagged and preserved behind FFI" means: the Rust crate contains a stub with the same signature that calls the original C/C++ implementation, which is kept compiled in. The migration is incomplete for that item, but the build and behavior are intact, and the report lists it.

## 9. Success metrics

### 9.1 Product-level metrics (reported per run)

| Metric                    | Definition                                                                                  |
| ------------------------- | ------------------------------------------------------------------------------------------- |
| Function coverage by tier | Percentage of functions translated at each memory-strategy tier (T0 to T4)                  |
| LOC coverage by tier      | Same, weighted by source lines                                                              |
| Unsafe density            | Count of `unsafe` blocks and `unsafe` lines per thousand Rust lines                         |
| Flagged count             | Functions and items preserved behind FFI stubs                                              |
| Behavioral agreement      | Differential-test pass rate across original test suite and generated characterization tests |
| Fuzz divergence count     | Number of input divergences found by differential fuzzing in a fixed budget                 |
| Runtime-check overhead    | Measured slowdown versus the original on the project's benchmarks                           |
| Determinism check         | Two runs produce identical output hashes                                                    |

### 9.2 Acceptance targets (guidance for milestones; adjustable via ADR)

- A "well-behaved" corpus program at L0 translates with 0 `unsafe` blocks in at least 80% of functions, 0 behavioral divergences on its test suite, and a deterministic report.
- A mixed C++/Rust build of a migrated module passes the original project's unmodified test suite.
- Run-to-run determinism holds across at least three consecutive runs and two machines.

These numbers are targets, not guarantees. The measured values for each run are always reported, whatever they are.

## 10. Positioning relative to alternatives

| Alternative                     | Strength                              | Weakness recast addresses                        |
| ------------------------------- | ------------------------------------- | ------------------------------------------------ |
| Manual rewrite                  | Highest idiom quality                 | Slow, expensive, introduces new bugs             |
| C2Rust-style transliteration    | Behavior-preserving, mature           | Output is `unsafe`; no safety gain by itself     |
| Ownership-guided research tools | Converts many pointers to safe types  | Mostly C-only, prototype-grade, no audit trail   |
| LLM-only translation            | Often produces idiomatic code quickly | Unreliable and unauditable on its own            |
| Do nothing and add sanitizers   | Zero migration cost                   | Finds bugs that happen; cannot rule out the rest |

recast's niche is **auditable, deterministic migration with evidence**, designed for customers who must justify the result to a regulator or auditor.
