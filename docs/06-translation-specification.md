# 06. Translation Specification

Translation turns TIR plus analysis facts into a Rust crate. This document defines the memory-strategy tiers, every mapping rule family, the runtime support crate, the output layout, and the idiom pass.

Status: Tiers T0 to T4 and the rules below are **Specified**. Generic recovery (section 12) is **Research-grade**. Concrete input/output pairs for every rule family are in `14-worked-examples.md`.

---

## 1. Overview of the process

For each item in the program (type, global, function):

1. **Decide the strategy** for every memory-related construct using the facts and the tier selection procedure (section 3).
2. **Build a Rust syntax tree** (an internal Rust AST, not text) by applying rules from the rule table.
3. **Emit** formatted Rust source, one module per source module, plus the crate scaffolding.
4. **Record** an explanation for every decision: rule ID, facts used, alternatives rejected.
5. **Hand off** to interop (shims for items that stay in C++ or are exported) and verification.

Rules are registered in a table; each rule has an ID (`TR-nnn`), a precondition over TIR and facts, a rewrite, a minimum fact confidence, and a list of verification obligations it creates.

---

## 2. Memory strategy tiers

Every pointer-like or memory-managing construct is translated using one of five tiers. The tier determines the safety guarantee, the cost, and how it is reported.

| Tier   | Name                   | What the output looks like                                                                                              | Safety guarantee                                                                                    | Runtime cost                                                 | `unsafe`                 |
| ------ | ---------------------- | ----------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------- | ------------------------------------------------------------ | ------------------------ |
| **T0** | Direct safe            | Owned values, `Box`, `Vec`, slices, `&T`, `&mut T`, `Option`                                                            | Compiler-proved                                                                                     | None beyond Rust's own bounds checks                         | None                     |
| **T1** | Index and handle based | `Vec<T>` arenas with `usize`/newtype handles instead of pointers                                                        | Compiler-proved (bounds-checked indexing)                                                           | Small (index bounds checks, indirection)                     | None                     |
| **T2** | Runtime-checked safe   | `Rc`/`Arc`, `Cell`, `RefCell`, `Mutex`/`RwLock`, atomics, checked cells                                                 | Memory-safe; violations of the original aliasing discipline become panics (or errors) instead of UB | Reference counting or borrow flag checks; measured in report | None                     |
| **T3** | Contained unsafe       | `unsafe` blocks, each small, each wrapped in a safe function with documented invariants and, where possible, assertions | Not compiler-proved; correctness rests on the documented invariant                                  | Minimal, assertions optional                                 | Yes, individually listed |
| **T4** | Flagged / preserved    | A Rust stub with the same signature calling the original C/C++ through FFI (`recast-interop`)                           | None gained; behavior unchanged                                                                     | FFI call overhead                                            | Boundary only            |

Rules:

- `#![forbid(unsafe_code)]` MUST be present at the top of every generated module whose items are all T0, T1 or T2. This is the compiler-enforced proof of "no unsafe here."
- T3 code MUST be isolated in modules named `*_unsafe` (or a nested `unsafe_impl` module) and MUST NOT call out to unrelated code while an invariant is temporarily broken.
- T4 stubs live in the `ffi` module group and are counted separately ("boundary unsafe").
- The tier of a **function** is the _maximum_ tier of any strategy inside it. The report shows both the per-function tier and the percentage of individual constructs at each tier.

---

## 3. Tier selection procedure

For each pointer-typed place or memory construct:

```
inputs: ownership, nullability, extent, mutability, region facts, escape, thread_shared, UB findings, config

1. If the construct is in the unsupported list (section 16) or has type Unknown      -> T4
2. If ownership == Owning and extent == Single and (Proven or Assumed)               -> T0 Box<T> (Option<Box<T>> if nullable)
3. If ownership == Owning and extent in {Fixed, Len, NulTerminated}                  -> T0 Vec<T> / Box<[T]> / [T; N] / CString-like
4. If ownership in {BorrowedShared, BorrowedMut} and no borrow conflict and region solvable -> T0 &T / &mut T / &[T] / &mut [T] (+ Option if nullable)
5. If ownership == SharedOwner                                                       -> T2 Rc<T>/Arc<T> (Arc if thread_shared or unknown thread safety)
6. If borrow conflict exists:
     a. data is Copy and single-threaded                                             -> T2 Cell<T>
     b. data not Copy, single-threaded                                               -> T2 RefCell<T>
     c. data shared across threads                                                   -> T2 Mutex<T>/RwLock<T>/atomics
     d. structure is graph-like and indices suffice                                  -> T1 arena + handles
7. If pointer arithmetic or casts remain unresolved and bounds can be asserted       -> T3 contained unsafe with checks
8. Otherwise                                                                         -> T4 flag the function (or T3 if configured `allow_unsafe = true` and the construct is local)
```

The first matching rule that is permitted by the configured **maximum tier** is chosen. If the best rule exceeds the maximum tier (for example the user set `max_tier = 2`), the enclosing function is flagged (T4) rather than silently using a higher tier.

Configuration knobs affecting selection (see document 09): `max_tier`, `prefer_index_over_rc`, `allow_arc_default`, `assume_malloc_succeeds`, `int_overflow`, `ub_policy`.

---

## 4. Type mapping

### 4.1 Primitive types

Target-dependent; derived from the `Target` record (document 04 section 1.3). The table shows the default LP64 Linux x86-64 case.

| C/C++                               | Rust                                                                                                                                                                              | Notes                                                                                |
| ----------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------ |
| `bool`                              | `bool`                                                                                                                                                                            |                                                                                      |
| `char`                              | `u8` when used as a byte or in text; `i8` when signed arithmetic on `char` is proven; `core::ffi::c_char` at FFI boundaries                                                       | Signedness of `char` is platform-defined; recast records the target setting          |
| `signed char` / `unsigned char`     | `i8` / `u8`                                                                                                                                                                       |                                                                                      |
| `short` / `unsigned short`          | `i16` / `u16`                                                                                                                                                                     |                                                                                      |
| `int` / `unsigned int`              | `i32` / `u32`                                                                                                                                                                     | Only if target `int` is 32 bits; otherwise fixed-width by `int_bits`                 |
| `long` / `unsigned long`            | `i64` / `u64` on LP64; `i32`/`u32` on LLP64 (Windows)                                                                                                                             | By target                                                                            |
| `long long` / `unsigned long long`  | `i64` / `u64`                                                                                                                                                                     |                                                                                      |
| `size_t` / `ssize_t` / `ptrdiff_t`  | `usize` / `isize`                                                                                                                                                                 |                                                                                      |
| `intN_t` / `uintN_t`                | `iN` / `uN`                                                                                                                                                                       | Direct                                                                               |
| `intptr_t` / `uintptr_t`            | `isize` / `usize`                                                                                                                                                                 | Flag arithmetic that treats them as pointers                                         |
| `float` / `double`                  | `f32` / `f64`                                                                                                                                                                     |                                                                                      |
| `long double`                       | Flagged (T4)                                                                                                                                                                      | x87 80-bit has no Rust primitive; if target `long double` is 64-bit it maps to `f64` |
| `wchar_t` / `char16_t` / `char32_t` | `u32` or `u16` by target / `u16` / `u32`                                                                                                                                          | Not Rust `char` (not all values are valid scalars)                                   |
| `void`                              | `()`                                                                                                                                                                              | `void*` is covered by pointer rules                                                  |
| `nullptr_t`                         | `()`/`None` contextual                                                                                                                                                            |                                                                                      |
| `enum` (unscoped)                   | Rust `enum` with explicit discriminants if all values used are enumerators and conversions from integers are checked; else a newtype `struct Name(i32)` with associated constants | The newtype preserves C semantics where out-of-range values flow in (`UB-016` aware) |
| `enum class`                        | Rust `enum` with `#[repr(underlying)]`, `TryFrom<underlying>` for conversions                                                                                                     |                                                                                      |

Named types (`typedef`, `using`) are preserved as `type Name = ...;` where the alias carries meaning (project-specific names), and elided for pure platform aliases (`size_t`).

### 4.2 Integer expression semantics

See document 05 section 10.4. Summary of emitted forms:

- Unsigned `+ - *` where overflow is possible: `wrapping_add`, `wrapping_sub`, `wrapping_mul`.
- Signed `+ - *`: plain operators, with `overflow-checks = true` in the generated release profile (default), or `wrapping_*` if `int_overflow = "wrap"`.
- Division and modulo: plain operators (Rust panics on zero divisor and on `MIN / -1`; the original is UB there).
- Shifts: `checked_shl`/`checked_shr` with panic on out-of-range count (`UB-007`), or plain when the count is a constant in range.
- Integer promotion and usual arithmetic conversions: TIR already contains explicit implicit-cast nodes; emit `as` casts at exactly those nodes so widths and signedness follow C++ rules.

### 4.3 Pointer and reference types (decision table)

Inputs: ownership, nullability, extent, mutability.

| Ownership                                     | Extent              | Nullable | Mutable | Rust type                                                                            |
| --------------------------------------------- | ------------------- | -------- | ------- | ------------------------------------------------------------------------------------ |
| Owning                                        | Single              | no       | n/a     | `Box<T>` (or `T` by value when `NoEscape` and not address-sensitive)                 |
| Owning                                        | Single              | yes      | n/a     | `Option<Box<T>>`                                                                     |
| Owning                                        | Fixed(n)            | no       | n/a     | `[T; N]` (small) or `Box<[T; N]>`                                                    |
| Owning                                        | Len / unknown count | no       | n/a     | `Vec<T>` (or `Box<[T]>` if never resized)                                            |
| Owning                                        | NulTerminated       | no       | n/a     | `CString`-like `ByteString` (section 9.1)                                            |
| BorrowedShared                                | Single              | no       | no      | `&T`                                                                                 |
| BorrowedShared                                | Single              | yes      | no      | `Option<&T>`                                                                         |
| BorrowedShared                                | Len / Fixed         | no       | no      | `&[T]`                                                                               |
| BorrowedMut                                   | Single              | no       | yes     | `&mut T`                                                                             |
| BorrowedMut                                   | Single              | yes      | yes     | `Option<&mut T>`                                                                     |
| BorrowedMut                                   | Len / Fixed         | no       | yes     | `&mut [T]`                                                                           |
| BorrowedShared/Mut                            | NulTerminated       | no       | by case | `&CStr` / `&[u8]` plus length handling                                               |
| SharedOwner                                   | Single              | any      | any     | `Rc<T>` / `Arc<T>`; `Weak<T>` for `std::weak_ptr` and non-owning back-pointers       |
| Unknown                                       | any                 | any      | any     | T2 or higher by selection procedure (not a raw pointer unless T3/T4)                 |
| Function pointer                              | n/a                 | no       | n/a     | `fn(Args) -> Ret` (`Option<fn(..)>` if nullable)                                     |
| Function pointer with user data (`void* ctx`) | n/a                 | any      | n/a     | Boxed closure `Box<dyn FnMut(..)>` when the pairing is provable; else T3/T4          |
| `void*` (opaque)                              | n/a                 | any      | n/a     | Typed by use after analysis (for example `Box<dyn Any>` or a concrete type); else T4 |
| Member pointer                                | n/a                 | n/a      | n/a     | Flagged unless trivially a field accessor                                            |

C++ references: `T&` -> `&mut T` or `&T` per mutability; `const T&` -> `&T`; `T&&` (rvalue references) -> by-value `T` at a move site; forwarding references handled per instantiation.

---

## 5. Aggregate types

### 5.1 Structs and classes

`struct`/`class` -> Rust `struct`. Field order and types are preserved. Access control:

- `public` -> `pub`
- `protected` -> `pub(crate)` (documented deviation; inheritance is by composition)
- `private` -> private (module-level). Rust privacy is per-module, so `friend` declarations map to `pub(super)`/`pub(crate)` as the minimal visibility that works; the report lists widened visibility.

Types crossing the FFI boundary (exported, or used in T4 stubs) MUST be `#[repr(C)]` and MUST have generated layout assertions (document 08 section 5).

### 5.2 Constructors, destructors, special members

- Constructors -> associated functions: default constructor -> `Default::default()` / `new()`; other constructors -> `new`, `with_*`, or `from_*` named by parameter types; each preserves the original member-initializer order.
- Destructors -> `impl Drop`. The generated `Drop` preserves destruction order (members in reverse declaration order, matching C++).
- Copy constructor / copy assignment -> `Clone` (`#[derive(Clone)]` when the member-wise copy is trivial and matches).
- Move constructor / assignment -> Rust move semantics. Where the original uses a moved-from object afterward (legal C++), analysis marks the use and translation inserts `std::mem::take`/`replace` to leave a valid default value in the source, matching C++ semantics.
- Deleted special members -> not implemented (types lack `Clone` etc.).
- Defaulted `operator==` etc. -> `#[derive(PartialEq, ...)]` when field-wise semantics match.

### 5.3 Unions

- A union whose active member is always tracked by an adjacent tag field (`enum`/`int` discriminant) -> Rust `enum` with data (T0).
- Otherwise a `#[repr(C)] union` (T3) with accessor functions marked `unsafe`, or flagged. Type punning through unions (`UB-009`) is flagged by default.

### 5.4 Bit-fields

Rust has no bit-fields. Generate a struct holding the backing integer(s) and accessor methods (`get_x`, `set_x`) implemented with shifts and masks that reproduce the compiler's layout from the `RecordLayout`. Layout-sensitive uses (FFI, serialization) require the layout assertions.

### 5.5 Inheritance and virtual functions

- **Single inheritance, no virtual functions**: composition (`base: Base` field) plus delegation methods; the derived type does not implement `Deref` to the base (deliberate; `Deref` for inheritance is a known anti-pattern).
- **Interfaces and virtual functions**: each polymorphic base becomes a `trait`; virtual methods become trait methods; derived types `impl Trait`; polymorphic pointers become `Box<dyn Trait>`, `&dyn Trait` or `Rc<dyn Trait>` by ownership.
- **Closed hierarchies** (whole-program mode, all derived classes known, no external derivation): MAY use an `enum` of the derived types with `match` dispatch (faster, no dynamic allocation) when configured (`closed_hierarchy_enum = true`).
- **Data in base classes with virtual functions**: base data is stored in a shared struct held by each derived type; trait methods access it via required accessor methods.
- **Multiple inheritance**: allowed only for interface-like bases (no data); multiple implementations of traits. Anything with shared data or virtual inheritance is flagged.
- **`dynamic_cast`**: translated using `Any`-style downcasting where the hierarchy is closed enough (`as_any()` helper in the trait) or via `match` on the enum form; otherwise flagged.
- **RTTI (`typeid`)**: supported only via `TypeId` of concrete types in closed settings; else flagged.

### 5.6 Nested types, namespaces

Namespaces -> Rust modules; nested classes -> sibling types in the enclosing module with qualified names (or an inner module named after the outer class). Using-directives are resolved and removed.

---

## 6. Functions

### 6.1 Naming

**[Decision]** Preserve source identifiers by default (`naming = "preserve"`), adding `#[allow(non_snake_case, non_camel_case_types, non_upper_case_globals)]` at the crate or module level. Reasons: traceability to the original source, stable FFI symbol names, and lower diff noise. The idiom pass may rename to Rust conventions (`naming = "rust"`) when no exported symbol is affected, or when exported names are preserved with `#[export_name = "..."]`. Rust keywords used as identifiers become raw identifiers (`r#type`).

### 6.2 Overloading

Rust has no overloading. Overloaded functions and methods receive deterministic suffixes derived from the **parameter types** (for example `area_f64`, `area_i32`), unless they are operator overloads that map to traits (section 6.4). The original name remains in the explanation record; exported FFI names follow `08` naming rules.

### 6.3 Default arguments

Default argument expressions are inlined at each call site (faithful, simple). The idiom pass MAY introduce builder or `Option`-based APIs for internal-only functions.

### 6.4 Operators

- Arithmetic, comparison, indexing, call, and dereference operators map to the matching `std::ops`/`std::cmp` traits where the signatures fit (`Add`, `AddAssign`, `Index`, `IndexMut`, `Neg`, `Not`, `PartialEq`, `PartialOrd`, `Deref`, `Fn*` for call operators on closures).
- Stream operators (`<<`, `>>` on streams) are translated per library mapping (section 14).
- `operator,`, `operator&&`/`||` (non-short-circuit overloads), `operator->` custom smart pointers, `operator new` / `delete` overloads: flagged unless a recognized smart-pointer pattern.

### 6.5 Parameters and return values

- Output parameters (`T*` or `T&` written by the callee) are kept as `&mut T` in the faithful output. The idiom pass MAY convert internal functions to return tuples.
- Return of a reference or pointer follows the lifetime rules (document 05 section 8).
- `noexcept`, `[[noreturn]]` -> `-> !` for `[[noreturn]]`; `noexcept` informs exception translation.
- `constexpr` -> `const fn` where possible.
- `inline`, `static` (file-local) -> private functions; `extern "C"` functions keep `#[no_mangle] pub extern "C"` when exported.

### 6.6 Variadic functions

- **Calls** to `printf`/`fprintf`/`sprintf`/`snprintf`/`scanf` with a literal format string are translated by parsing the format at translation time (section 14.4).
- Other variadic calls (for example `execl`) -> T4.
- **Definitions** of variadic functions -> T4.

### 6.7 Recursion and stack

Recursive functions translate directly. The report notes recursion depth risks; Rust's stack limit is not larger than C++'s, so no new risk is introduced, but deeply recursive `Drop` on long singly-linked lists is a known hazard in Rust: generated `Drop` implementations for recursive owning structures MUST be iterative (see example 3 in `14-worked-examples.md`).

---

## 7. Memory management constructs

| C/C++                                               | Rust (tier)                                                                | Notes                                                                                                                        |
| --------------------------------------------------- | -------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------- |
| `new T(args)` assigned to owning pointer            | `Box::new(T::new(args))` (T0)                                              |                                                                                                                              |
| `new T[n]`                                          | `vec![T::default(); n]` or `Vec::with_capacity` + init (T0)                | Zero-initialization difference: `new T[n]` leaves scalars uninitialized (UB to read); recast initializes (`UB-005` recorded) |
| `delete p` / `delete[] p`                           | Dropping the owner (end of scope, `drop(p)`, reassignment)                 | Explicit `drop` emitted only when the original order of destruction matters                                                  |
| `malloc(n * sizeof(T))`                             | `Vec<T>` (zero-initialized) or `Box<[T]>`                                  | `calloc` -> `vec![0; n]`                                                                                                     |
| `realloc`                                           | `Vec::resize` or `reserve`                                                 |                                                                                                                              |
| `free(p)`                                           | Drop                                                                       |                                                                                                                              |
| `memcpy(dst, src, n*sizeof(T))` with proven extents | `dst[..n].copy_from_slice(&src[..n])` (`Copy` types) or `clone_from_slice` | Overlap is `UB-017`; `memmove` -> `copy_within`                                                                              |
| `memset(p, v, n)`                                   | `slice.fill(v)` for element-size-1 types, else a loop or typed fill        |                                                                                                                              |
| `std::make_unique<T>(args)`                         | `Box::new(T::new(args))`                                                   | `unique_ptr` may be null: `Option<Box<T>>` unless proven non-null                                                            |
| `std::make_shared<T>`                               | `Rc::new` / `Arc::new`                                                     |                                                                                                                              |
| `std::weak_ptr<T>`                                  | `Weak<T>`                                                                  |                                                                                                                              |
| Placement new, custom allocators, `std::pmr`        | T4                                                                         |                                                                                                                              |
| RAII (destructors on scope exit)                    | `Drop` plus scoping                                                        | Destruction order of locals matches C++ (reverse declaration)                                                                |
| `std::move`                                         | By-value move                                                              | See 5.2                                                                                                                      |

Allocation failure: `malloc` returning `NULL` is modeled by `assume_malloc_succeeds` (default `false`: the result is `Option` and unhandled null cases panic with a recorded `UB-006`-class site). `new` throwing `bad_alloc` is mapped per the exception rules (section 10.4); Rust's default allocation failure aborts, which is documented as a behavioral difference when the original catches `bad_alloc`.

---

## 8. Strategies in detail (T1 and T2)

### 8.1 T1: arenas and handles

For graph-like structures (doubly-linked lists, trees with parent pointers, graphs):

```rust
pub struct Arena<T> { items: Vec<Option<T>> }
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Handle<T> { idx: u32, gen: u32, _p: core::marker::PhantomData<fn() -> T> }
```

- Handles carry a generation counter so stale handles are detected (turning use-after-free into a checked error, not UB).
- Pointer fields of type `T*` into the structure become `Option<Handle<T>>`.
- Requires that all objects of the type are allocated in the arena (closed world for that type); otherwise T2.
- `recast_rt::arena` provides the implementation.

### 8.2 T2: runtime-checked sharing

- `Rc<T>` / `Arc<T>` for shared ownership.
- `Rc<RefCell<T>>` / `Arc<Mutex<T>>` for shared **mutable** ownership.
- `Cell<T>` for `Copy` data accessed through aliased `&` references (zero overhead).
- Borrow-flag violations (`RefCell` double mutable borrow) panic with a message naming the original source location (recorded via `#[track_caller]` helpers in `recast_rt::checked`); each occurrence in testing is an indication that the original relied on a pattern whose safety the analysis could not show.
- **[Research]** This tier corresponds to the approach used by Cpp2Rust (runtime-enforced ownership and mutability checks to reconcile C++ aliasing with Rust's ownership model), whose measured overhead ranged from about 2% on one benchmark to about 6x on a pointer-arithmetic-heavy one. recast's report measures and publishes its own overhead for each run.

---

## 9. Strings, text and the runtime support crate

### 9.1 Strings [Decision]

C++ strings are byte sequences; Rust `String`/`&str` must be valid UTF-8. Mapping them directly would turn a non-UTF-8 byte into a panic or corruption.

- `std::string` -> `recast_rt::ByteString` (a thin wrapper over `Vec<u8>` with a string-like API: `push_str`, `find`, `substr`, `c_str`, comparison, `+`, and so on).
- `const std::string&` and `std::string_view` -> `&[u8]`-backed `&ByteStr` (`recast_rt::ByteStr`).
- `const char*` strings with `NulTerminated` extent -> `&CStr` / `CString`.
- Promotion to `String`/`&str`: allowed when **all** producers and consumers of a given string place are proven to hold UTF-8 (for example string literals only passed to formatting) or when `assume_utf8 = true` is set (default `false`, recorded as an assumption).
- String literals: `"abc"` as `const char*` -> `b"abc\0"` as `&CStr` or `&[u8]` by use; as `std::string` initializer -> `ByteString::from(b"abc")`.
- Narrow, wide and UTF-16/32 strings map to `Vec<u16>`/`Vec<u32>` wrappers of the same family.

### 9.2 The runtime support crate `recast-rt`

Shipped as a normal crate that generated code depends on. Contents (each module documented and tested; `#![forbid(unsafe_code)]` except `ffi` and `raw`):

| Module        | Purpose                                                               | Key items                                                                                                                                                      |
| ------------- | --------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `bytes`       | Byte-string family                                                    | `ByteString`, `ByteStr`, `CStrExt` helpers                                                                                                                     |
| `arena`       | T1 support                                                            | `Arena<T>`, `Handle<T>` with generation checks                                                                                                                 |
| `checked`     | T2 support with source-located panics                                 | `CheckedCell<T>`, `CheckedRc<T>`, `#[track_caller]` borrow helpers                                                                                             |
| `ub`          | UB-site handling                                                      | `ub::trap(UbKind, &'static SourceLoc) -> !`, `ub::checked_*` helpers; behavior configurable at build time via feature (`panic` default, `abort` for no-unwind) |
| `numeric`     | C-compatible numeric helpers                                          | `wrapping_*` helpers, checked shifts, float-to-int conversions with documented behavior                                                                        |
| `exception`   | Exceptions-as-values                                                  | `CppException` enum: `Logic(String)`, `InvalidArgument`, `OutOfRange`, `Runtime`, `BadAlloc`, `Other(Box<dyn Any + Send>)` etc.                                |
| `stdio`       | C stdio shims                                                         | Safe wrappers for `FILE` operations implemented over `std::fs`/`std::io`; `printf`-style formatting helpers for format strings that are not constants          |
| `collections` | Behavior-preserving collections                                       | `OrderedMap` (comparator-driven, wraps `BTreeMap` with a key wrapper), `StableSort`, `UnstableSort`                                                            |
| `ffi`         | **The only module with `unsafe`** used by T4 stubs and exported shims | `extern "C"` helpers, `catch_unwind` guards, buffer conversion helpers, allocator bridge (`recast_alloc`/`recast_free`)                                        |
| `raw`         | Contained unsafe utilities for T3                                     | Bounds-asserting raw-pointer wrappers                                                                                                                          |

Policy: each `unsafe` block in `recast-rt` has a `// SAFETY:` comment stating the invariant and a test. The crate's unsafe count is reported separately from the translated code's.

---

## 10. Control flow

### 10.1 Structured statements

`if`, `while`, `do-while` (`loop { ...; if !cond { break } }`), `for` (see below), and labeled `break`/`continue` map directly.

C `for (init; cond; step)` -> `{ init; while cond { body; step } }`, with care for `continue`: a `continue` inside the body must run `step`, so when the body contains `continue`, emit `loop { if !cond {break} ; 'body: { ...body... } step }` with `continue` becoming `break 'body`. The idiom pass converts recognized counting loops to `for i in a..b`.

### 10.2 `switch`

- Cases without fallthrough -> `match` with integer/enum patterns; `default` -> `_`.
- Fallthrough cases -> duplicate the fallthrough tail into the preceding arm if small, or use a state variable with a `loop { match state { ... } }` otherwise. Each form records the original case labels in comments.
- `switch` containing declarations or Duff's-device-like structure (jumps into nested blocks) -> flagged.

### 10.3 `goto`

- Forward `goto` to a label at the end of the function used for cleanup (`goto fail;`) -> labeled block with `break 'cleanup` (the common pattern).
- Backward `goto` forming a loop -> `loop` with labeled `continue`.
- Other reducible control flow -> restructured with a labeled-block/`loop` scheme (a relooper-style algorithm over the CFG).
- Irreducible control flow -> a `loop { match state { ... } }` state machine, which is correct but unreadable; reported; or flagged by config.

### 10.4 Exceptions

**[Decision]** Exceptions are translated to `Result` where _analyzable_, and to unwinding panics at boundaries where not.

- **Analyzable**: a function whose `throw` sites and the call graph below it can be enumerated (types known, all `catch` handlers visible in the module) -> `Result<T, E>` where `E` is an enum of the exception types that can escape, or `CppException`. `throw X` -> `return Err(X.into())`; `try/catch` -> `match`; calls that may throw use `?`. Destructors run correctly because Rust unwinds by early return.
- **Not analyzable** (exceptions crossing into unknown code, `catch(...)` with rethrow of unknown types): the function is translated with `panic_any`/`catch_unwind` using `recast_rt::exception`, and the construct is recorded (T2-class runtime behavior); or flagged by config.
- **`noexcept` violations** (a throw escapes a `noexcept` function): C++ calls `std::terminate`; translation emits `abort`, matching.
- **Across FFI**: exceptions and panics MUST NOT cross the boundary (document 08 section 4.4).

---

## 11. Expressions

- **Evaluation order**: C++17 fixes some orders (right operand of `<<`, assignments) and leaves function-argument order unspecified. Rust evaluates left-to-right. Where TIR marks `eval_order: Unspecified` and the sub-expressions have conflicting side effects, translation hoists sub-expressions into temporaries in the order the _reference compiler build_ produces (determined by the exporter's target compiler configuration, default Clang), records `UB-021`/unspecified-order notes, and verifies by tests.
- **Assignment as expression** (`a = b = c`, `if ((x = f()) != 0)`): hoist into a block expression or statement sequence.
- **Increment/decrement as expressions**: `i++` in expression position -> `{ let t = i; i += 1; t }`.
- **Comma operator**: sequence in a block expression.
- **Conditional `?:`**: `if c { a } else { b }`.
- **Short-circuit** `&&`, `||`: same operators.
- **Compound assignments**: same, with the integer rules of 4.2.
- **Casts**: C-style cast -> the minimal Rust cast that reproduces the _actual_ C++ conversion (TIR classifies the cast kind): integer widening/narrowing `as`; pointer-to-pointer casts are handled by ownership facts (a `T*` to `U*` cast with `reinterpret_cast` semantics is `UB-009`-class and flagged unless layout-compatible); `const_cast` removal of const is flagged unless it's a no-op for the analysis; `static_cast` to a base class -> field access/trait object coercion.
- **sizeof/alignof**: constants from the `Target` layout (also asserted in generated layout tests for FFI types).
- **Literals**: integer suffixes map to typed literals; octal/hex/binary preserved; character literals to `u8` or `i8` (`b'a'`); float literals preserved.
- **Comparisons of pointers**: `p == q` between pointers translated per their Rust forms (`std::ptr::eq` for references, `Rc::ptr_eq` for shared owners, handle equality for T1).
- **Null pointer comparisons**: `p == NULL` -> `p.is_none()` for `Option` forms.

---

## 12. Templates and generics (Research-grade refinement)

### 12.1 Baseline: monomorphic translation

Each used instantiation is translated as concrete code. Names are derived from the template name plus the type arguments (for example `max_i32`, `Stack_f64`). This is always correct and may duplicate code.

### 12.2 Generic recovery (optional pass)

For an instantiation family, if all instantiations are structurally identical after substituting type arguments for type-dependent positions, and the type parameters are used only through operations expressible as Rust trait bounds, emit a generic Rust item with those bounds and rewrite call sites. Required inference: map C++ operations on `T` (`+`, `==`, `<`, copy, default construction, destruction) to trait bounds (`Add<Output=T>`, `PartialEq`, `PartialOrd`, `Clone`, `Default`). Items using SFINAE, specialization, non-type template parameters beyond simple integers (which map to const generics), or variadic templates remain monomorphic.

### 12.3 Standard-library templates

Instances of standard containers are mapped by the library table (section 14), not translated from their library source.

---

## 13. Preprocessor output

- **Constants** (`ObjectConstant`, `ObjectExpr`) -> `pub const NAME: Type = value;` (type inferred from use sites or from the macro's literal suffix; ambiguity resolved by the most common use type with a note).
- **Function-like macros**: by classification (document 05 section 7): `#[inline] fn`, `macro_rules!`, or expanded.
- **Conditional compilation**: `#[cfg(feature = "...")]` and `#[cfg(target_...)]` per document 05 section 6; the generated `Cargo.toml` declares the features.
- **Includes**: removed; modules and `use` statements replace them. Header-only items shared across TUs become one item in a shared module.
- **Pragmas and attributes**: `#pragma once` removed; `#pragma pack` -> `#[repr(C, packed)]` (flag taking references to packed fields: Rust forbids unaligned references, so such accesses use `read_unaligned` helpers or copies); other pragmas carried as comments and noted.

---

## 14. Standard library mapping

All mappings are defined in `models/cxx-std.toml` (recast's library database) and the corresponding runtime crate code. A mapping lists: Rust type, constructors, methods, behavioral differences, and verification notes. Items not in the table are T4 (flagged) unless a contract says otherwise.

### 14.1 Containers

| C++                                               | Rust                                                                                                  | Behavioral notes recorded in report                                                                                                                                                                                                 |
| ------------------------------------------------- | ----------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `std::vector<T>`                                  | `Vec<T>`                                                                                              | `operator[]` out of range: UB -> Rust panics. Iterator invalidation: Rust prevents it at compile time; patterns that rely on it are flagged. `vector<bool>` is a special bit-packed type: flagged or mapped to a bit-vector wrapper |
| `std::array<T, N>`                                | `[T; N]`                                                                                              |                                                                                                                                                                                                                                     |
| `std::deque<T>`                                   | `VecDeque<T>`                                                                                         |                                                                                                                                                                                                                                     |
| `std::list<T>`                                    | `LinkedList<T>` or `Vec` by usage; stable iterators across mutation are flagged                       |                                                                                                                                                                                                                                     |
| `std::map<K, V, Cmp>`                             | `recast_rt::OrderedMap` (a `BTreeMap` with comparator wrapper)                                        | Ordering by comparator `Cmp` must be a strict weak ordering; mapping verifies comparator is expressible as `Ord`                                                                                                                    |
| `std::set<T, Cmp>`                                | `OrderedSet` similarly                                                                                |                                                                                                                                                                                                                                     |
| `std::unordered_map`/`unordered_set`              | `HashMap`/`HashSet` with a deterministic hasher in test builds                                        | Iteration order differs from C++ and is unspecified in both languages; characterization tests MUST NOT depend on it; report notes sites that iterate and produce observable output                                                  |
| `std::pair`, `std::tuple`                         | `(A, B)`, tuples (or named struct if fields accessed by `first`/`second` frequently, for readability) |                                                                                                                                                                                                                                     |
| `std::optional<T>`                                | `Option<T>`                                                                                           |                                                                                                                                                                                                                                     |
| `std::variant<...>`                               | Rust `enum` generated per use                                                                         |                                                                                                                                                                                                                                     |
| `std::span<T>`                                    | `&[T]` / `&mut [T]`                                                                                   |                                                                                                                                                                                                                                     |
| `std::string_view`                                | `&ByteStr`                                                                                            |                                                                                                                                                                                                                                     |
| `std::stack`, `std::queue`, `std::priority_queue` | `Vec`, `VecDeque`, `BinaryHeap` (with comparator wrapper)                                             | Heap tie-order among equal elements may differ; noted                                                                                                                                                                               |
| `std::bitset<N>`                                  | `recast_rt::BitSet<N>`                                                                                |                                                                                                                                                                                                                                     |

### 14.2 Algorithms

| C++                                                                                                                                                 | Rust                                                               |
| --------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------ |
| `std::sort`                                                                                                                                         | `sort_unstable_by` with comparator (C++ `sort` is not stable)      |
| `std::stable_sort`                                                                                                                                  | `sort_by` (stable)                                                 |
| `std::find`, `find_if`, `any_of`, `all_of`, `count`, `accumulate`, `transform`, `for_each`, `copy`, `fill`, `reverse`, `min_element`, `max_element` | Iterator adapters and slice methods                                |
| `std::unique`, `std::remove`, `std::erase`                                                                                                          | `Vec::dedup_by`, `retain`, etc., with erase-remove idiom collapsed |
| `std::binary_search`, `lower_bound`, `upper_bound`                                                                                                  | `partition_point` / `binary_search_by`                             |
| `std::swap`, `std::move`, `std::forward`                                                                                                            | `mem::swap`, moves, per-instantiation forwarding                   |

Elements of `std::sort`-like results that depend on instability (equal elements' relative order) are noted: unstable sorts are implementation-defined in both languages and may differ between compilers; tests relying on that are flagged as fragile.

### 14.3 Smart pointers and utilities

See section 7. `std::function` -> `Box<dyn Fn...>` / `Rc<dyn Fn...>`; `std::bind` -> closures; `std::chrono` -> `std::time` types with unit-preserving wrappers; `std::random` engines -> **bit-exact** reimplementation of the specific engine (`mt19937` and so on) in `recast_rt` where determinism matters, else flagged (the Rust `rand` crate does not reproduce C++ sequences).

### 14.4 I/O and formatting

- **`printf`-family with literal formats**: parse the format at translation time. Supported conversions: `d i u x X o c s f e g p %` with flags, width, precision, length modifiers. Map to `print!`/`println!`/`format!`/`write!` with equivalent format specs where Rust's formatter reproduces the output exactly; otherwise use `recast_rt::stdio::cformat` which implements C semantics byte-for-byte (for example `%g`, `%a`, `%p`). Argument-type mismatches with the format (a classic `UB`) are findings.
- **Non-literal formats** -> `recast_rt::stdio::cformat_dyn` (interprets format at run time) or T4 by config.
- **iostream** (`std::cout << x`) -> `write!` to `stdout`/`stderr` handles, with `std::endl` flushing semantics preserved; formatting manipulators (`std::setw`, `std::hex`, and so on) map to format specs or to `recast_rt::iofmt` helpers.
- **`FILE*`** -> `recast_rt::stdio::File` (owning handle with C-like API) when the pointer is owning and used with a recognized set of functions; otherwise T4.
- **`std::fstream`, `std::ifstream`, `std::ofstream`** -> `std::fs::File` wrappers with buffered readers/writers.
- **`errno`** -> thread-local `recast_rt::errno` updated by shim functions that model it.

### 14.5 Concurrency

| C++                                            | Rust                                                                                                                                                                                                                        |
| ---------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `std::thread`                                  | `std::thread::spawn` (closure must be `Send + 'static`; borrowed captures use `thread::scope`)                                                                                                                              |
| `std::mutex`, `std::lock_guard`, `unique_lock` | `Mutex<T>` + guard; where the original keeps a mutex _separate_ from the data it protects, the mapping associates the mutex with the data it guards (determined by analysis) or uses `Mutex<()>` plus documented discipline |
| `std::shared_mutex`                            | `RwLock`                                                                                                                                                                                                                    |
| `std::condition_variable`                      | `Condvar`                                                                                                                                                                                                                   |
| `std::atomic<T>`                               | `AtomicXxx` with the same memory ordering                                                                                                                                                                                   |
| `std::future`, `std::async`, `promise`         | `std::thread` + channels or `JoinHandle`; flagged for complex use                                                                                                                                                           |
| `pthread_*`                                    | Same mappings via recognized patterns; otherwise T4                                                                                                                                                                         |
| `thread_local`                                 | `thread_local!`                                                                                                                                                                                                             |

### 14.6 Global and static state

- `static` locals with dynamic init -> `OnceLock` / `LazyLock` (thread-safe init; matches C++11 semantics).
- Mutable globals: single-threaded-only proof -> `thread_local! { Cell/RefCell }`; otherwise `Mutex`/atomics. `static mut` is **never** emitted outside T3.
- Initialization order across translation units (the static initialization order fiasco): constant-initialized globals become `const`/`static`; others are initialized lazily on first use, which changes _when_ initialization occurs; recorded, because it can change observable behavior if constructors have side effects.

---

## 15. Idiom pass

The first output is **faithful** (correct, close to the structure of the original). The idiom pass produces an **idiomatic** variant. Both are verified; the idiomatic variant is only delivered if it passes the same checks.

| ID      | Transformation                                                                      | Precondition                                                                                          |
| ------- | ----------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------- |
| IDM-001 | Index-based counting loops -> `for i in a..b`                                       | Loop var not modified in body; bounds loop-invariant                                                  |
| IDM-002 | Loops over slice/vec with index only for element access -> `for x in &v` / `iter()` | No other uses of the index                                                                            |
| IDM-003 | Hand-written sum/min/max/find loops -> iterator adapters                            | Pattern match plus identical overflow semantics                                                       |
| IDM-004 | `if p != NULL { ... }` -> `if let Some(p) = p { ... }`                              | `Option` form                                                                                         |
| IDM-005 | Manual `memcpy` loops -> `copy_from_slice`                                          | Equal proven lengths, non-overlapping                                                                 |
| IDM-006 | C-style error codes -> `Result<T, E>`                                               | Opt-in; changes API (internal functions only, or with exported wrappers preserving the old signature) |
| IDM-007 | Out-parameters -> tuple returns                                                     | Internal functions only                                                                               |
| IDM-008 | Removal of redundant `as` casts and redundant temporaries                           | Type-checked equivalence                                                                              |
| IDM-009 | Renaming to Rust conventions                                                        | `naming = "rust"` and no exported name changes (or `export_name` preserved)                           |
| IDM-010 | Replace `ByteString` with `String` where UTF-8 proven                               | Section 9.1 conditions                                                                                |
| IDM-011 | Collapse simple macro-generated code into functions/generics                        | Round-trip check passes                                                                               |
| IDM-012 | Remove T2 runtime checks made unnecessary after earlier simplifications             | Re-run analysis proves exclusivity                                                                    |

Every transformation:

- has a rule ID,
- lists its preconditions and an argument that it preserves behavior,
- is individually switchable in configuration,
- generates a verification obligation (differential tests on the affected functions).

---

## 16. Constructs that are flagged (T4)

Default unsupported set (always flagged, preserved via FFI, listed in the report):
inline assembly; `setjmp`/`longjmp`; computed `goto`; user-defined variadic functions; placement `new` and custom allocator machinery; `operator new`/`delete` overloads outside recognized patterns; member pointers (non-trivial); multiple/virtual inheritance with data; unions with unproven discipline; `volatile` access to hardware registers; `long double` on targets where it is 80-bit; coroutines; types or functions containing `Unknown`; code the UB policy rejected; items depending on compiler-specific extensions without a mapping (for example some `__attribute__`s and intrinsics, with a recognized subset of intrinsics mapped to `core::arch` in a later milestone).

### 16.1 T4 stub generation

For each flagged function `f`:

1. The interop stage generates a C-ABI wrapper in C++ (`extern "C" R recast_wrap_f(args)`) that calls the original `f` and converts exceptions to status codes.
2. The Rust crate gets a function with the **same Rust signature the translation would otherwise have produced**, implemented as a safe wrapper over the `extern "C"` declaration (the wrapper is in the `ffi` module group).
3. Callers in translated code call it like any other function, so a single flagged function does not infect its callers.
4. The build links the original C++ object for flagged items.

Argument and result conversion at the boundary uses the ownership conventions in document 08 section 4.

---

## 17. Output crate layout

```
<output-root>/
  Cargo.toml                 // workspace or single crate, edition, features from config atoms, profile settings
  rust-toolchain.toml        // pinned toolchain
  recast-manifest.json         // tool version, input hashes, rule-set hash, tier map summary
  src/
    lib.rs                   // module tree mirrors source layout; crate-level lints
    <module>/...rs           // one Rust module per source module (directory or namespace group)
    ffi/                     // exported shims and T4 stubs (the only place boundary unsafe lives)
    unsafe_impl/             // T3 code, if any
  include/                   // generated C headers for exported API (document 08)
  tests/                     // generated differential and characterization tests
  benches/                   // benchmarks for overhead measurement
  README.generated.md        // what this crate is, tier summary, how to rebuild
```

`Cargo.toml` template (key parts):

```toml
[package]
name = "<crate_name>"
version = "<from config>"
edition = "2021"
rust-version = "<MSRV>"

[lib]
crate-type = ["rlib", "staticlib"]      # staticlib only if exported to C/C++

[features]
default = [ <default-on features from configuration atoms> ]
use_fast = []                           # one per feature atom

[dependencies]
recast-rt = { version = "=<exact>", default-features = false }

[profile.release]
overflow-checks = true                  # when int_overflow = "panic"
panic = "abort"                         # when the crate is linked into C/C++ (document 08)
```

Lint header in each module (T0 to T2): `#![forbid(unsafe_code)]`. Crate-level: `#![deny(warnings)]` is **not** emitted by default (generated code may legitimately produce warnings); `clippy` is run with a configured set in verification.

Source comments: every translated function carries `/// recast: translated from <file>:<line> [rules: ...] [tier: Tn]`; original comments attached to AST nodes (as exposed by Clang's raw-comment support) are carried over as ordinary comments.

---

## 18. Explanation records

For every item and every tier decision, write a JSON record under `explain/`:

```json
{
	"item": "fn:c:@F@sum#*1I#l#",
	"rule_ids": ["TR-201", "TR-310"],
	"tier": "T0",
	"choices": [
		{
			"construct": "param a",
			"chosen": "&[i32]",
			"because": [
				"OWN-004 Proven BorrowedShared",
				"ARR-003 Proven Len(n)",
				"NUL-002 Likely NonNull"
			],
			"rejected": [
				{ "strategy": "*const i32 (T3)", "reason": "higher tier than needed" }
			]
		}
	],
	"assumptions": ["NUL-002 Likely NonNull (all 3 call sites pass non-null)"],
	"ub_sites": [],
	"verification_obligations": ["differential:sum", "callers-adapted:3"]
}
```

`recast explain <item>` renders these.

---

## 19. Rule identifier scheme

| Prefix                        | Meaning                                    |
| ----------------------------- | ------------------------------------------ |
| `TR-0xx`                      | Type mapping                               |
| `TR-1xx`                      | Items, structures, classes                 |
| `TR-2xx`                      | Pointers, references, ownership strategies |
| `TR-3xx`                      | Statements and control flow                |
| `TR-4xx`                      | Expressions                                |
| `TR-5xx`                      | Library mapping                            |
| `TR-6xx`                      | Preprocessor and configuration             |
| `IDM-nnn`                     | Idiom pass                                 |
| `OWN-`, `NUL-`, `ARR-`, `UB-` | Analysis rules (document 05)               |

Every rule in the rule table MUST have an entry in `14-worked-examples.md` or a test that demonstrates it.
