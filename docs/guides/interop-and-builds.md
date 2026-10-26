# 08. Interop and Build Integration

Real migrations are incremental. For months or years, some of a codebase is C/C++ and some is Rust, and the two must call each other correctly. This subsystem generates and checks the boundary, and integrates the Rust output into existing builds.

Status: the C-ABI shim approach is **Specified**. Richer C++ interop (templates and classes across the boundary) is **Outlined**.

**[Research]** The boundary between old and new code is a research topic of its own. The PLDI 2026 program lists "&inator: Correct, Precise C-to-Rust Interface Translation" as a separate paper, and Google reports that safe interoperability between Rust and C++ is a core difficulty of its gradual transition strategy. recast treats the boundary as a first-class, verified artifact rather than glue.

---

## 1. Goals and constraints

- **G-I1.** Any module can be migrated independently while the rest stays in C/C++.
- **G-I2.** Existing C/C++ callers of the module continue to compile and link **without source changes** wherever possible (public headers preserved).
- **G-I3.** The boundary is generated, layout-checked, and tested; no hand-written glue is required for supported constructs.
- **G-I4.** All boundary-related `unsafe` is confined to the `ffi` module group and counted separately in the report.
- **G-I5.** Neither exceptions nor panics cross the boundary.
- **G-I6.** The migration is reversible: a build switch selects the original or the Rust implementation of each module.

Constraint: Rust and C++ share no stable ABI beyond C. **[Decision]** The boundary is the **C ABI**. C++ classes, templates and overloading are not directly callable from Rust and vice versa; they are adapted by generated C-ABI shims.

---

## 2. Boundary directions

### 2.1 Direction A: C/C++ calls Rust (a migrated module is used by remaining C++)

For each exported function or type of the migrated module:

1. **Rust side**: an `extern "C"` function with a stable symbol name, in `src/ffi/exports.rs`:

```rust
#[no_mangle]
pub extern "C" fn mylib_parse(buf: *const u8, len: usize, out: *mut Parsed) -> i32 {
    ffi_guard(|| {
        // SAFETY: caller guarantees buf is valid for len bytes (contract: buf non-null, len given)
        let input = unsafe { recast_rt::ffi::slice_from_raw(buf, len)? };
        let out_ref = unsafe { recast_rt::ffi::mut_from_raw(out)? };
        crate::parse::parse(input, out_ref)
    })
}
```

`ffi_guard` (in `recast_rt::ffi`) wraps `catch_unwind`, converts `Result`/panics to status codes, and sets `errno`-style error info retrievable through `mylib_last_error()`. 2. **C++ side**: the original public header is kept (so callers do not change). Its declarations are backed by a **generated forwarding layer**:

- Free functions: the header's inline definitions or a generated `.cpp` forwards to the `extern "C"` function.
- Classes: a **forwarding class** with the original interface that holds an opaque handle to the Rust object and forwards each method to a generated `extern "C"` function (a pimpl pattern); constructors call `mylib_X_new`, the destructor calls `mylib_X_free`, copy semantics call `mylib_X_clone` when the Rust type is `Clone`, and moves transfer the handle.
- Layout-visible structs (plain data used directly by callers) are kept as `#[repr(C)]` types with identical layout, shared by header generation.

3. **Header generation**: `include/<name>.h` produced by a cbindgen-style generator from the `extern "C"` surface, plus the compatibility layer for the original header.

### 2.2 Direction B: Rust calls C/C++ (flagged items, unmigrated dependencies)

1. **C++ side**: a generated `extern "C"` wrapper per needed function/method:

```cpp
extern "C" int32_t recast_wrap_decode(const uint8_t* buf, size_t len, Out* out, RecastErr* err) {
  try { *out = decode(std::span<const uint8_t>(buf, len)); return 0; }
  catch (const std::exception& e) { recast_set_err(err, e.what()); return 1; }
  catch (...) { recast_set_err(err, "unknown exception"); return 2; }
}
```

2. **Rust side**: an `extern "C"` declaration in `src/ffi/imports.rs` plus a **safe wrapper function** with the intended Rust signature that checks preconditions, converts arguments, calls the import, and converts errors to `Result`. C-header imports of plain C APIs MAY use `bindgen`-style generation; C++ APIs go through the generated wrapper layer; `cxx` MAY be used where its type subset fits (opt-in).
3. **Build**: wrappers are compiled into a small `recast_shims` library alongside the original objects.

### 2.3 Header-only and template-heavy code

Code that lives entirely in headers as templates cannot be exported through a C ABI (there is no single instantiation). Options, in preference order:

1. Migrate **all users** of the template together (dependency-closed unit).
2. Export explicit instantiations (specific type arguments) through shims.
3. Keep the template in C++ and migrate its callers only.
   The migration planner (`recast scan`) marks such libraries as `header-only-template` and suggests these options.

---

## 3. Ownership and data conventions at the boundary

Boundary rules prevent the most common source of FFI memory bugs: confusion over who frees what.

| Situation                                  | Convention                                                                                                                                                                                                                                  |
| ------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Borrowed pointer + length** (input data) | `(const T* ptr, size_t len)` -> `&[T]` on the Rust side; callee does not retain it beyond the call unless the contract says `retained` (then the API must provide a release function)                                                       |
| **Mutable buffer**                         | `(T* ptr, size_t len)` -> `&mut [T]`; exclusive for the call                                                                                                                                                                                |
| **Out parameter**                          | `T* out` non-null -> `&mut T` (must be initialized memory of type `T`; for non-trivial `T`, use construct/destroy functions)                                                                                                                |
| **Ownership transfer to Rust**             | Pointer allocated by a documented allocator; the Rust side must know how to free it. Default: both sides use `malloc`/`free` for plain buffers; for Rust-allocated objects handed to C++, use the exported `T_free` function (never `free`) |
| **Ownership transfer to C++**              | Rust allocates with its allocator and exports `T_free`; or uses `recast_alloc`/`recast_free` (thin wrappers over the system allocator) when the C++ side will call `free`                                                                   |
| **Opaque handles**                         | `typedef struct X X;` with `X_new`, `X_free`, and method functions; Rust type behind a `Box` raw pointer (`Box::into_raw` / `Box::from_raw`), null checks on every entry                                                                    |
| **Strings**                                | `(const char* nul_terminated)` -> `&CStr`; or `(const uint8_t*, size_t)` pairs for byte strings; returned strings are either static, caller-allocated buffers (`buf, cap` -> returns required size), or owned handles with a free function  |
| **Callbacks**                              | `(fn_ptr, void* user_data)` pairs; Rust closures are boxed behind `user_data` and a trampoline `extern "C" fn`; the API MUST define the lifetime of `user_data` and the thread on which callbacks run                                       |
| **Error reporting**                        | Return `int32_t` status (0 ok); detailed message via an out-parameter or `*_last_error()` (thread-local)                                                                                                                                    |
| **Nullable pointers**                      | Every pointer parameter's nullability is declared in the contract; Rust shims check and return an error status or map to `Option`                                                                                                           |
| **Variadics**                              | Not supported across the boundary                                                                                                                                                                                                           |
| **Booleans**                               | `bool` has C++/Rust layout agreement (1 byte, 0 or 1); values other than 0/1 are UB in Rust: shims accept `uint8_t` and validate                                                                                                            |
| **Enums**                                  | Passed as fixed-width integers; converted with `TryFrom` and error on unknown values                                                                                                                                                        |
| **Unions**                                 | Not supported across the boundary (flagged)                                                                                                                                                                                                 |
| **Thread safety**                          | The contract states whether functions are reentrant and whether objects may cross threads; Rust types exposed must be `Send`/`Sync` accordingly; otherwise documented single-thread                                                         |

Every convention instance is generated from the **contract** (document 09) or derived from analysis facts; where neither is available the shim generator refuses to generate the export and reports "contract required for parameter `x`."

---

## 4. Safety rules for generated shims

### 4.1 Allocation

Memory allocated by one side is freed by the same side's deallocator. The generator tracks allocation provenance in the contract and refuses combinations that cross allocators.

### 4.2 Null and validity checks

Every raw pointer received by an `extern "C"` export is checked for null (unless contracted non-null _and_ the profile allows trusting contracts) and for alignment. Violations return an error status instead of invoking UB (and optionally log).

### 4.3 Lifetimes

References created from raw pointers inside shims have lifetimes limited to the shim function body. Anything retained beyond the call is copied or converted to an owned type.

### 4.4 Panics and exceptions

- **Rust panics** never unwind into C++. Generated exports use `catch_unwind` at the boundary (requires `panic = "unwind"`) or the crate is built with `panic = "abort"` (in which case a panic aborts the process; documented). The default for crates exported to C/C++ is `panic = "abort"` for simplicity and predictability, configurable.
- **C++ exceptions** never propagate into Rust. Every C++ wrapper has `try { ... } catch (...) { ... }` returning status codes. If the original function throws and the Rust caller expects exception semantics, the safe wrapper converts the status to `Err(CppException)`.
- `extern "C-unwind"` is out of scope in the first release.

### 4.5 Reentrancy and callbacks

A callback invoked from C++ into Rust code that then calls back into C++ is supported provided shims keep no non-reentrant global state; the generator avoids globals in shims.

### 4.6 `unsafe` accounting

All `unsafe` created for the boundary lives in:

- `src/ffi/exports.rs`, `src/ffi/imports.rs`, and `recast_rt::ffi`.
  These files are annotated and counted under "ffi_boundary_blocks" in the report. Translated business logic MUST NOT contain boundary `unsafe`.

---

## 5. Layout verification

For every type that crosses the boundary or is shared as plain data:

**C++ side** (generated):

```cpp
static_assert(sizeof(Parsed) == 40, "recast layout");
static_assert(alignof(Parsed) == 8, "recast layout");
static_assert(offsetof(Parsed, version) == 0, "recast layout");
static_assert(offsetof(Parsed, flags) == 4, "recast layout");
```

**Rust side** (generated):

```rust
const _: () = assert!(core::mem::size_of::<Parsed>() == 40);
const _: () = assert!(core::mem::align_of::<Parsed>() == 8);
const _: () = assert!(core::mem::offset_of!(Parsed, version) == 0);
const _: () = assert!(core::mem::offset_of!(Parsed, flags) == 4);
```

The expected values come from the exporter's `RecordLayout` for the target. A mismatch fails the build, never at run time.

Rules for shared types:

- `#[repr(C)]` always; no `Option<Box<T>>` or other niche-optimized types in the shared layout except pointer-sized `Option<NonNull<T>>`/`Option<&T>` where the null-pointer optimization is guaranteed for the specific form.
- `#[repr(C, packed)]` only when the C++ struct is packed; field references to packed fields are forbidden in generated code (use copies).
- Bit-fields: shared as the backing integer plus accessors on both sides.
- Padding bytes are not part of the contract; if serialization depends on them (memcmp of structs), the shim generator flags it.

---

## 6. Build-system integration

### 6.1 Input: the compilation database

The primary input is `compile_commands.json`, produced by CMake (`-DCMAKE_EXPORT_COMPILE_COMMANDS=ON`), Bear (for Make), Bazel's compilation-database extractors, or Meson/Ninja (built-in support). Clang tooling uses this file to reproduce each file's exact flags. If none exists, `recast scan --generate-db` attempts to produce one by observing a build (for example with Bear) and records that choice.

### 6.2 Output: how the Rust crate is built

- The generated crate is built with Cargo as a **`staticlib`** (default for C/C++ consumers) and optionally `cdylib` or `rlib`.
- `panic` strategy and `overflow-checks` are set in the crate's profile; they MUST match what the consuming C++ link expects (all Rust code in one link unit uses the same panic strategy).
- Toolchain pinned by `rust-toolchain.toml`; MSRV recorded.

### 6.3 Integrating with the existing build

| Build system     | Integration                                                                                                                                                                                                                                            |
| ---------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| CMake            | Generate `cmake/recast_<module>.cmake` using the **Corrosion** module (imports Cargo crates as CMake targets); the module's C++ sources are removed from the target and replaced by the imported Rust static library plus generated forwarding sources |
| Bazel            | Generate `BUILD.bazel` rules using `rules_rust` (`rust_static_library`) and `cc_library` for shims                                                                                                                                                     |
| Make / Autotools | Generate a `recast.mk` fragment that invokes `cargo build`, then adds the static library and shim objects to the link line                                                                                                                             |
| Meson            | Generate a `subproject` or custom target invoking Cargo                                                                                                                                                                                                |
| Custom           | `recast export-build --template custom` produces the list of objects, flags and libraries so the user's build can be adapted                                                                                                                           |

All integrations expose a switch:

```
-DRECAST_USE_RUST_<MODULE>=ON|OFF
```

When `OFF`, the original C++ sources build as before. This is how A/B comparison and rollback are achieved (G-I6).

### 6.4 Link details

- Symbol visibility: exports use `#[no_mangle]` and explicit export lists; internal symbols are hidden.
- Link order: Rust static libraries typically need system libraries (`-ldl -lpthread -lm` on Linux); `recast export-build` obtains the list from `cargo rustc -- --print native-static-libs` and records it.
- Duplicate runtime concerns: multiple Rust static libraries in one process duplicate the Rust standard library; the integration produces **one** Rust static library per final binary (workspace merged) or uses `cdylib` boundaries.
- LTO across languages is an optional optimization when LLVM versions match (Clang and rustc built against compatible LLVM); not required.
- Cross-compilation: the target triple recorded in the `Target` MUST match the Rust target triple used; the tool refuses to generate the build integration if they disagree.

---

## 7. Incremental migration protocol

The unit of migration is a **module**: a set of translation units, headers and tests with an identifiable dependency boundary.

### 7.1 Planning (`recast scan` and `recast plan`)

1. Build the include and call graph; compute module candidates (directory, library target, or user-defined groups).
2. Rank modules for migration order using: dependency leafness (fewest outgoing dependencies first), size, test coverage, share of constructs by feasibility tier (from a quick analysis pass), and exposure (public API breadth).
3. Emit a **migration plan**: ordered modules, per-module feasibility estimate (percent T0/T1/T2/T4 predicted), contracts needed, risky constructs, and blockers.

### 7.2 Execution loop per module

1. **Contracts**: author or review `recast-contracts.toml` entries for the module's API (the tool proposes a draft from analysis with confidence labels).
2. **Translate** the module (dependencies already migrated are used as Rust crates; unmigrated dependencies are accessed through Direction B shims).
3. **Generate shims and headers** (Direction A for the module's exports).
4. **Swap** in the build behind `RECAST_USE_RUST_<MODULE>`.
5. **Link-swap tests**: run the original test suite against the Rust-backed build.
6. **Differential and fuzz verification** (document 07).
7. **Gate** per profile; review the report.
8. **Soak**: optionally run in **shadow mode** (below) in a pre-production environment.
9. **Cut over**: set the switch default to ON; keep the original sources until sign-off; then remove them.

### 7.3 Shadow mode (testing aid)

A build option where each exported function is implemented as: call original, call Rust, compare observable results, log divergences, and return the original's result. Used in staging to compare on real traffic. Never enabled in production by default. It requires both implementations in the same binary, hence the C-ABI shims on both sides.

### 7.4 Rollback

Setting the switch to OFF restores the original behavior. The Rust crate remains in the repository, unused.

### 7.5 Ordering rule

Migrate **bottom-up** over the module dependency graph where possible: leaves first. This minimizes Direction B shims (a migrated module calls already-migrated Rust dependencies directly). When cycles exist, migrate the strongly connected group together or place the cut at the narrowest interface.

---

## 8. Naming conventions at the boundary

| Item                         | Convention                                                                                                       |
| ---------------------------- | ---------------------------------------------------------------------------------------------------------------- |
| Exported function            | `<lib>_<module>_<name>` (for overloaded C++ functions, `<lib>_<module>_<name>_<paramtypes>`); stable across runs |
| Opaque type                  | `<Lib><Type>` with `_new`, `_free`, and `_<method>` functions                                                    |
| Wrapper in C++ (Direction B) | `recast_wrap_<mangled-unique-name>`                                                                              |
| Error retrieval              | `<lib>_last_error()` returning a thread-local C string                                                           |
| Allocation bridge            | `recast_alloc`, `recast_free`                                                                                    |

Names are derived from the TIR `ItemId` and stable settings so they do not change when unrelated code changes.

---

## 9. Testing the boundary

Generated tests per exported function:

1. **Round-trip test**: call through the C ABI with typical arguments; compare with calling the Rust function directly.
2. **Null/invalid test**: pass null and boundary lengths; assert error status, no crash.
3. **Ownership test**: allocate and free across the boundary under AddressSanitizer; assert no mismatched free.
4. **Panic/exception containment test**: force a panic (via test hook) and a C++ exception; assert a status code results.
5. **Layout test**: the `static_assert`/`const` assertions of section 5 are compiled in both directions.
6. **Thread test** (if the contract says thread-safe): concurrent calls under ThreadSanitizer.

---

## 10. Limitations and honest scope

- C++ virtual dispatch across the boundary (Rust implementing a C++ interface, or C++ implementing a Rust trait) is **Outlined**: initial support covers C++ classes _wrapping_ Rust implementations (pimpl-forwarding) and Rust calling C++ through wrappers; cross-language inheritance is a later milestone.
- Template-heavy public APIs may force whole-library migration (section 2.3).
- Exceptions are converted to status codes; code that relies on exception types crossing the boundary needs the status-code convention adapted on both sides.
- Performance: each crossing costs a function call plus conversions; fine-grained cross-boundary calls in hot loops can slow things down. The planner flags such call patterns and suggests migrating the caller and callee together.
