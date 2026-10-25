# 11. Roadmap and Milestones

An ordered build plan with exit criteria. No people or dates are assigned; sizes are relative (S/M/L/XL) so a builder can sequence work. Each milestone ends with something runnable and measurable.

Principle: **vertical slices first.** Get a tiny C function through every stage (export, TIR, analysis, translate, verify, report) before widening any stage.

Status: **Specified**.

---

## Milestone map

| #    | Milestone                         | Size | Outcome                                                               |
| ---- | --------------------------------- | ---- | --------------------------------------------------------------------- |
| M0   | Foundations and skeleton          | M    | Repo, CI, schemas, a `hello` function crosses the whole pipeline      |
| M1   | C subset translator (L0)          | L    | Pure functions and simple structs, no heap; T0 only; V0 to V2         |
| M2   | Heap and pointers (L1 C)          | XL   | Ownership, nullability, extent analyses; Box/Vec/slices; T0 to T2     |
| M3   | Verification and gate             | L    | Full ladder V0 to V4, gate profiles, report                           |
| M4   | Preprocessor and configurations   | L    | Macro classification, `#[cfg]` mapping, round-trip checks             |
| M5   | Interop and incremental migration | L    | Shims both ways, link-swap tests, CMake integration                   |
| M6   | C++ core (L2)                     | XL   | Classes, RAII, inheritance to traits, exceptions, std containers      |
| M7   | Templates and generics            | L    | Instantiation families, generic recovery                              |
| M8   | Hardening and scale               | L    | Corpus, performance, mutation testing, determinism at scale           |
| M9   | Release 1.0                       | M    | Docs, packaging, benchmarks published                                 |
| M10+ | Extensions                        | open | Optional AI proposer layer, V5, richer interop, more languages levels |

---

## M0 Foundations and skeleton (M)

**Build**

- Cargo workspace with the crates from document 03 (empty but compiling), `xtask`, lint and format config, pinned toolchain.
- `recast-schema`: `recast-ast/1` types with serialization and schema validation.
- Exporter v0 (`exporter/`, C++ against Clang LibTooling): emit functions, basic types, statements and expressions for C; read `compile_commands.json`.
- `recast-ir`: TIR types, validator (the ten invariants in document 04), dump format, ID scheme.
- `recast-frontend`: AST to TIR for the L0 subset.
- `recast-translate`: emit Rust for integer functions.
- `recast-cli`: `init`, `export`, `analyze`, `translate`, `version`, `doctor`.
- `recast-testkit`: snippet harness, golden runner.
- CI "fast" pipeline.

**Exit criteria**

- `int add(int a, int b)` and the `sum(const int*, size_t)` example cross the pipeline to formatted Rust that compiles.
- Determinism check passes (two runs identical).
- Schema round-trip tests pass.

**Risks**: Clang version coupling (pin it); schema churn (version early and freeze `/1` only at M3).

---

## M1 C subset translator, level L0 (L)

**Scope**: scalar types, arithmetic with the integer rules (document 06 section 4.2), control flow (`if`, loops, `switch` without fallthrough), structs by value, fixed arrays, string literals, pure functions, `static` functions, enums, constants.

**Build**

- Complete TIR expression/statement coverage for the subset; derived CFG; liveness; definite assignment (`UB-005`).
- Integer semantics and casts; `UB-001`, `UB-007`, `UB-008` detection by interval analysis.
- Translation rules `TR-0xx`, `TR-3xx`, `TR-4xx` for the subset; naming; layout asserts for `#[repr(C)]` structs.
- Goldens for each rule; L4 behavioral differential on snippets; L5 property-based on arithmetic.
- `recast-verify` V0, V1 and V2 (whole-program tests, characterization for pure functions).

**Exit criteria**

- 100% of L0 corpus (teaching programs) translates at T0 with 100% differential pass.
- Property tests on integer arithmetic run 10^6 cases without divergence.
- Mutation kill rate on the L0 subset >= 95% at V2.

---

## M2 Heap and pointers (XL)

**Scope**: `malloc`/`free`, pointers to single objects and arrays, linked lists and trees, `const` correctness, string handling with `char*`, output parameters.

**Build**

- Points-to analysis (document 05 section 2) with Steensgaard fallback.
- Ownership constraints (section 3) via `ConstraintSolver` (Z3 backend), unsat-core handling, summaries (section 11).
- Nullability, array extent with relational bounds, lifetime regions, escape analysis.
- Library models for libc (`models/libc.toml`).
- Tier selection (document 06 section 3); T0 to T2 forms; `recast-rt` modules `bytes`, `arena`, `checked`, `ub`, `numeric`.
- `UB-002`, `UB-003`, `UB-004`, `UB-006`, `UB-012`, `UB-015`.
- L3 ground-truth suite; L7 soundness validation harness.
- Idiom pass v1 (`IDM-001` to `IDM-005`, `IDM-008`).

**Exit criteria**

- `cJSON` translates with >= 70% of functions at T0/T1, <= 5% flagged, 100% differential pass on its test suite.
- Zero L7 soundness contradictions.
- Vulnerability regression corpus: all bugs found or neutralized per policy.
- Analysis of 100k lines completes in under 5 minutes on a reference machine (target; recorded).

**Risks**: ownership analysis precision on real code (mitigation: tiers and contracts); solver time (mitigation: budgets, per-function solving).

---

## M3 Verification and gate (L)

**Build**

- V3 differential fuzz harness generation, V4 sanitizer/Miri integration, sandbox runner.
- Link-swap test machinery stubs (full in M5).
- Safety gate with profiles and gate rules (document 07 section 4).
- Report schema `recast-report/1`, `report.md`, SARIF.
- Evidence classes; claim-language lint (CI fails if the report generator emits forbidden wording).
- `verify`, `run`, `gate`, `report`, `explain`, `diff` commands.
- Mutation testing harness (document 10 section 5).
- Freeze schemas `/1`.

**Exit criteria**

- Mutation kill rate >= 98% across V2 to V4 on the M2 corpus; surviving mutants reviewed.
- Report passes schema validation; limitations statement present.
- Determinism check (GATE-005) works on corpus.
- `recast run` is the documented default workflow.

---

## M4 Preprocessor and configurations (L)

**Build**

- Exporter: macro definitions, expansion events, macro trail on AST nodes, conditional-directive map.
- Configuration modes `single`, `listed`, `derived` (document 05 section 6) with symbolic activation conditions and atom classification.
- Macro taxonomy, hygiene checks, round-trip check, `macro_rules!` and `fn` emission; include-guard removal; `#[cfg]`/Cargo feature emission.
- Report metric: macro preservation.

**Exit criteria**

- `zlib` translates for the listed configurations with `#[cfg]` where expressible; builds for each configuration; tests pass.
- > = 80% of function-like macro uses in the corpus preserved as abstractions where classification permits; 0 round-trip violations accepted.

---

## M5 Interop and incremental migration (L)

**Build**

- Direction A and B shim generation (document 08), contracts schema and loader, `contracts suggest`.
- Layout verification (both sides), boundary test generation.
- Build integration: CMake (Corrosion) first, then Bazel, Make.
- Link-swap testing (V2a), shadow mode.
- `scan`, `plan`, `export-build` commands.
- T4 stub generation and degrade-to-T4 repair.

**Exit criteria**

- A medium C library migrates one module at a time behind `RECAST_USE_RUST_<MODULE>`; original test suite passes unchanged in both settings.
- Boundary tests pass under ASan/TSan; layout asserts hold.
- Flagged-function stubs work end to end (a `setjmp` function stays in C and is called from Rust).

---

## M6 C++ core, level L2 (XL)

**Scope**: classes and structs, constructors/destructors/RAII, single inheritance and virtual functions as traits, operator overloading, references and move semantics, namespaces, exceptions-as-Result, standard containers and strings, iostream and `printf`, smart pointers.

**Build**

- Exporter: C++ AST coverage (classes, templates instantiations, lambdas, overload resolution results).
- TIR: class hierarchy view, member initializer order, special members.
- Translation rules `TR-1xx`, library mapping (document 06 section 14): vectors, maps, strings (`ByteString`), smart pointers, algorithms, formatting (`cformat`).
- Exception analysis and `Result` translation.
- Models `cxx-std.toml`.
- Ownership analysis extended to smart pointers and `shared_ptr` (`SharedOwner`).

**Exit criteria**

- `tinyxml2` and a representative class-hierarchy corpus translate; >= 60% functions at T0/T1/T2 without flags; differential pass 100%.
- Behavioral-difference notes (unordered iteration, sort instability) emitted as specified.
- Exception-heavy snippets pass L4.

---

## M7 Templates and generics (L)

**Build**

- Instantiation families, structural-equality check, trait-bound inference, generic emission; const generics for integer parameters.
- `constexpr` to `const fn`.
- SFINAE detection (`monomorphic_only`).

**Exit criteria**

- `fmt` subset and a container corpus: >= 50% of instantiation families recovered as generics where eligible, remainder monomorphic; all differential tests pass.
- Recovered generics produce identical behavior on all original instantiations.

---

## M8 Hardening and scale (L)

**Build**

- Performance: parallel per-module analysis; memory profiling; incremental cache; solver tuning.
- Full corpus (document 10 section 3); known-hard corpus assertions; vulnerability corpus.
- Fuzzing of the tool (L8) campaigns; crash triage.
- Overhead benchmarking protocol and published numbers.
- Idiom pass v2 (`IDM-006`, `IDM-007`, `IDM-009` to `IDM-012`).
- Security review of the tool (sandbox, input handling, supply chain), dependency pinning and `cargo-vet`/`cargo-deny`.

**Exit criteria**

- 1M-line project analysis under 30 minutes on reference hardware (target).
- Zero P0 bugs open; zero false accepts in corpus; zero determinism failures.
- Mutation kill rate target met.

---

## M9 Release 1.0 (M)

**Build**

- Documentation set finalized from this directory; user guide and migration playbook.
- Packaging: static binaries, container image, `recast doctor` checks.
- Benchmarks and evidence tables published with methodology.
- Optional attestation signing.
- Release pipeline with reproducible builds.

**Exit criteria**: all release criteria (document 10 section 6).

---

## M10+ Extensions (not scheduled)

1. **AI proposer layer** (ADR-003): propose contracts, lifetime annotations, idiom rewrites; every proposal passes the full ladder; deterministic only through recorded outputs; off by default.
2. **V5**: Kani/refinement-checking for pure functions.
3. **Richer interop**: cross-language virtual dispatch, `cxx` integration, `extern "C-unwind"`.
4. **L3 constructs**: selected intrinsics to `core::arch`, `volatile` MMIO patterns, coroutines.
5. **IDE integration**: LSP showing tiers and findings inline.
6. **Fix-it mode**: suggest source changes in the original C++ that would raise the tier (for example "add length parameter").
7. **Additional front ends**: other C-family dialects; Objective-C is out of scope.
8. **Regulated-industry packs**: evidence templates mapped to ISO 26262, DO-178C, IEC 62304 workflows.

---

## Dependency graph

```
M0 -> M1 -> M2 -> M3 -> M5 -> M6 -> M7 -> M8 -> M9
              \-> M4 -----^
```

M4 may proceed in parallel with M3 once M2 exists; M5 requires M3 (verification) and benefits from M4.

---

## Scope control rules

1. No milestone starts a new language feature until the previous milestone's exit criteria hold.
2. A feature that cannot be verified is flagged (T4), not shipped as translated.
3. Anything outside the language levels in document 01 is flagged by default, whatever the pressure to support it.
4. Performance work never weakens determinism.
5. A false accept is a stop-the-line event.
