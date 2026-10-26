# 12. Risks, Decisions and Open Questions

Three registers in one file: what can go wrong, what has been decided (and why), and what is still open.

Status: **Specified** (living document; update when reality changes).

---

## 1. Risk register

Likelihood (L) and Impact (I): H / M / Lo. Each risk has a mitigation built into the design, and a trigger that tells the builder the risk is materializing.

### 1.1 Technical risks

| ID   | Risk                                                                                    | L   | I        | Mitigation                                                                                                                      | Trigger                                          |
| ---- | --------------------------------------------------------------------------------------- | --- | -------- | ------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------ |
| R-01 | Ownership analysis too imprecise on real code, so most functions land in T2 or T4       | H   | H        | Tiers (T1/T2 fallbacks), contracts, array-extent analysis, compiler-as-oracle refinement, per-function solving                  | T0 share < 40% on corpus after M2                |
| R-02 | Solver blow-up (time or memory) on large programs                                       | M   | H        | Per-function and per-field-path solving, summaries, logical step limits, fixed seeds, Steensgaard fallback                      | Any run exceeds budget; `solver_unknowns` rising |
| R-03 | A wrong translation passes the gate (false accept)                                      | M   | Critical | Verification ladder, mutation testing of the verifier, L7 soundness checks, forbidden-claims lint, conservative defaults        | Any false accept: stop the line                  |
| R-04 | Behavior differences caused by UB replacement surprise customers                        | M   | H        | UB catalogue, explicit policy (`report`/`strict`/`reject`), every site listed with original vs translated behavior              | Customer reports divergence on a UB site         |
| R-05 | Runtime overhead of T2 too high for performance-sensitive code                          | M   | M        | Measure and publish; `prefer_index_over_rc`; `Cell` for `Copy` data; tier map shows hotspots; user can raise `max_tier` or flag | Overhead > 2x on a benchmark                     |
| R-06 | Macro-heavy code explodes or loses meaning                                              | H   | M        | Macro taxonomy, round-trip check, expansion fallback, preservation metric                                                       | Preservation < 50% on corpus                     |
| R-07 | Configuration explosion (`#ifdef` combinatorics)                                        | M   | M        | Symbolic activation conditions, atom cap, `single` default, fallbacks with findings                                             | `max_atoms` exceeded frequently                  |
| R-08 | Clang version drift breaks the exporter                                                 | M   | M        | Pin Clang; versioned schema; exporter conformance tests; upgrade procedure                                                      | New Clang release                                |
| R-09 | Rust toolchain drift changes output behavior                                            | Lo  | M        | Pinned `rust-toolchain.toml` recorded in manifest; MSRV tested                                                                  | Toolchain bump                                   |
| R-10 | Non-determinism sneaks in (hash ordering, parallelism, solver)                          | M   | H        | Sorted containers, fixed seeds, GATE-005 on every run, determinism tests L9                                                     | Any determinism failure: P0                      |
| R-11 | Template and `constexpr` complexity swamps C++ support                                  | H   | M        | Monomorphic baseline always correct; generic recovery optional; `monomorphic_only`                                              | M7 slips                                         |
| R-12 | Library mapping gaps (STL breadth)                                                      | H   | M        | Table-driven models; unmapped items flagged, not guessed; prioritize by corpus frequency                                        | Flag reasons dominated by library gaps           |
| R-13 | Semantic mismatch in standard-library behavior (sort stability, hash order, formatting) | M   | M        | Behavioral notes in report; bit-exact `printf` and RNG reimplementations; tests avoid unspecified orders                        | Differential failures on formatting              |
| R-14 | Strings: non-UTF-8 data corrupts or panics                                              | M   | H        | `ByteString` default; `String` only when proven or assumed (`assume_utf8`)                                                      | UTF-8 panics in tests                            |
| R-15 | Exceptions: incomplete analysis changes control flow                                    | M   | H        | `Result` only where analyzable; otherwise panic/unwind with boundary rules; flag                                                | Exceptions crossing unknown code                 |
| R-16 | Concurrency: data-race semantics not captured                                           | M   | H        | Thread-sharing analysis, `Arc`/`Mutex` forms, TSan on the original, flag unrecognized sync                                      | TSan findings in original                        |
| R-17 | FFI boundary bugs (double free, layout mismatch)                                        | M   | H        | Contracts, generated shims, layout asserts both sides, boundary tests under ASan                                                | Boundary test failures                           |
| R-18 | Generated code unreadable, so teams will not adopt it                                   | M   | H        | Faithful vs idiomatic variants, idiom pass, provenance comments, naming preserve option                                         | User feedback; idiomaticity metric               |
| R-19 | Verification cost too high for large projects                                           | M   | M        | Profiles, incremental cache, execution-count budgets, link-swap reuse of existing tests                                         | Run times > CI tolerance                         |
| R-20 | Projects with weak tests give weak evidence                                             | H   | M        | Characterization recording, fuzzing, `unexercised` listing, honest evidence classes                                             | Coverage < 50%                                   |
| R-21 | Verifier harness itself is buggy (tests pass vacuously)                                 | M   | H        | Mutation testing, canary tests that must fail, review of comparer functions                                                     | Mutation survivors                               |
| R-22 | Security: tool executes untrusted code (tests, builds)                                  | M   | H        | Sandbox mandatory (document 07 section 9); no network; resource limits                                                          | Any sandbox escape                               |

### 1.2 Product and business risks

| ID   | Risk                                                                                        | L   | I   | Mitigation                                                                                                                                                    |
| ---- | ------------------------------------------------------------------------------------------- | --- | --- | ------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| B-01 | Well-funded or government-backed competitors (DARPA TRACTOR performers, GrammaTech, others) | H   | H   | Differentiate on verified evidence, incremental migration, reports usable for compliance; focus on a niche first (embedded or security-sensitive C libraries) |
| B-02 | Free alternative (C2Rust plus manual work, open research tools) satisfies low-end users     | H   | M   | Target buyers who need evidence and support; keep faithful output free of surprises                                                                           |
| B-03 | LLM-based translation improves fast and bypasses rule-based approach                        | M   | H   | The architecture is LLM-compatible: LLMs may propose, recast verifies. The verifier and report are the durable moat                                           |
| B-04 | Customers expect "push button, done"                                                        | H   | M   | Clear scoping (`scan`, `plan`), honest feasibility estimates, tiers, no inflated claims                                                                       |
| B-05 | Long sales cycles in regulated industries                                                   | H   | M   | Start with open-source and developer-led adoption; evidence packs later                                                                                       |
| B-06 | Liability: customer ships mistranslated code                                                | M   | H   | Claim language rules, limitations statement, contractual scope, UB listing                                                                                    |
| B-07 | Dependence on LLVM/Clang licensing and community direction                                  | Lo  | Lo  | Apache-2.0 with LLVM exception; stable LibTooling                                                                                                             |
| B-08 | Standards shift (memory-safety mandates change)                                             | M   | M   | Product value does not depend on one mandate; migration and evidence remain needed                                                                            |

---

## 2. Architecture decision records (ADRs)

Format: Context, Decision, Consequences. IDs are stable and referenced elsewhere.

### ADR-001: Source-to-source translator, not a new language

- **Context.** Options included a new language with built-in migration, a compiler backend, or a translator.
- **Decision.** Build a translator plus verifier plus migration helper that emits ordinary Rust.
- **Consequences.** Output is usable with all existing Rust tooling; no ecosystem bootstrapping; value comes from translation quality and evidence.

### ADR-002: Use Clang as the frontend through a schema-versioned exporter

- **Context.** C/C++ semantics (preprocessing, overload resolution, templates) are too complex to reimplement.
- **Decision.** A small C++ exporter on LibTooling emits `recast-ast/1`; everything else is in Rust and independent of Clang.
- **Consequences.** Accuracy and compatibility come from Clang; Clang version is pinned; the Rust core can be tested with schema fixtures.

### ADR-003: No AI in the core translation path

- **Context.** LLMs can propose idiomatic rewrites but are nondeterministic and can be wrong in subtle ways. Research (for example C2SaferRust) shows LLM post-processing can reduce raw pointers, but gains are not guaranteed correct.
- **Decision.** Core pipeline is deterministic and rule-based. An optional AI layer (M10+) may only **propose** hypotheses (contracts, annotations, rewrites). Every proposal is accepted only if the deterministic pipeline and verification ladder approve it. The AI layer is off by default and cannot lower a gate requirement.
- **Consequences.** Reproducibility and auditability preserved; AI can improve results without becoming a trust dependency; the verifier is the product's moat.

### ADR-004: Tiered memory strategies instead of all-or-nothing

- **Context.** Some C++ patterns map to safe Rust exactly, some need runtime checks, some cannot be proven.
- **Decision.** Tiers T0 to T4 (document 06) with a max-tier knob and reporting per tier.
- **Consequences.** The tool is useful on real code from day one; customers see exactly what they got; safe-subset claims are precise.

### ADR-005: Typed, tree-shaped TIR with side fact tables and derived CFG

- **Context.** Rust is structured; C++ has `goto` and macros; analyses need CFGs; translation needs structure.
- **Decision.** Keep a structured tree as the primary IR, derive the CFG for analyses, store facts in side tables keyed by `NodeId`.
- **Consequences.** Translation stays close to source structure; analyses are decoupled; irreducible control flow handled by the relooper/state-machine fallback.

### ADR-006: Conservative defaults (may-alias, maybe-null, unknown)

- **Context.** An unsound "no alias" claim can produce wrong or unsafe output.
- **Decision.** Absent proof, assume the unsafe-side fact; wrong conservatism costs performance only.
- **Consequences.** Correctness over speed; precision work is optimization, not a correctness dependency.

### ADR-007: Preserve macros as abstractions when round-trip checks pass

- **Context.** Preprocess-first translation loses abstractions and configuration structure (Hayroll findings).
- **Decision.** Classify macros; emit functions or `macro_rules!` when a round-trip check proves structural equivalence; else expand.
- **Consequences.** More readable output; extra frontend complexity; expansion fallback keeps correctness.

### ADR-008: Strings as bytes by default

- **Context.** C++ strings are byte arrays; Rust `String` requires UTF-8.
- **Decision.** `ByteString`/`ByteStr` family by default; `String` when UTF-8 is proven or `assume_utf8` is set.
- **Consequences.** No hidden panics on binary data; idiom pass upgrades where safe.

### ADR-009: C ABI as the only interop boundary

- **Context.** No stable C++/Rust ABI exists; template and overloading cannot cross.
- **Decision.** All crossings use generated C-ABI shims; classes cross via opaque handles and forwarding classes.
- **Consequences.** Robust and toolchain-independent; some APIs need whole-library migration; richer interop is a later extension.

### ADR-010: Verification is independent of analysis

- **Context.** If the verifier trusts analysis facts, an analysis bug hides itself.
- **Decision.** Verification treats facts as untrusted and checks behavior of the original against the output.
- **Consequences.** Higher cost, much higher trust; mutation testing keeps the verifier honest.

### ADR-011: Faithful output first, idiomatic variant second

- **Context.** Making output pretty and making it correct pull in opposite directions.
- **Decision.** Produce a faithful translation, verify it, then run a separate switchable idiom pass whose result must pass the same verification.
- **Consequences.** A correct baseline always exists; idiom bugs are isolated and attributable.

### ADR-012: Determinism as a hard requirement

- **Context.** Auditors and CI need reproducible output.
- **Decision.** Same inputs and configuration produce byte-identical outputs; checked on every run.
- **Consequences.** Sorted containers, seeded solvers, no wall-clock in outputs, parallelism that does not affect results.

### ADR-013: Z3 behind a `ConstraintSolver` trait

- **Context.** Ownership inference is naturally a constraint problem; Crown demonstrates scalability with Z3.
- **Decision.** Use Z3 initially; hide behind a trait so another SAT/SMT solver or a custom solver can replace it.
- **Consequences.** Fast start; no hard lock-in; solver version pinned for determinism.

### ADR-014: Preserve identifiers by default

- **Context.** Renaming harms traceability and FFI stability.
- **Decision.** Keep original names with lint allowances; rename only in the idiom pass when no exported symbol changes.
- **Consequences.** Output looks like C in places; diffs against the original stay readable.

### ADR-015: Unsigned arithmetic wraps explicitly; signed overflow panics by default

- **Context.** C++ unsigned wraps; signed overflow is UB; Rust release builds wrap silently unless checks enabled.
- **Decision.** Emit explicit `wrapping_*` for unsigned, rely on `overflow-checks = true` for signed (configurable).
- **Consequences.** Behavior is independent of build profile for unsigned; signed overflow becomes visible instead of silent.

### ADR-016: Language levels and an always-flagged list

- **Context.** Full C++ is unbounded; trust requires clear scope.
- **Decision.** Levels L0 to L3 and a named list of constructs always flagged (T4) (document 01 and 06 section 16).
- **Consequences.** Scope is honest; flagged items still build and run via original code.

---

## 3. Open questions

Each has a recommended default so the builder is never blocked.

| ID   | Question                                                                         | Options                                                                | Recommended default                                                                                      | Resolve by |
| ---- | -------------------------------------------------------------------------------- | ---------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------- | ---------- |
| Q-01 | Which Clang/LLVM version to pin first?                                           | Latest stable at start vs LTS-style older                              | Latest stable at project start; upgrade only at milestone boundaries                                     | M0         |
| Q-02 | Rust edition and MSRV for generated code                                         | 2021 vs 2024                                                           | 2021 for widest compatibility; MSRV recorded; test 2024 later                                            | M0         |
| Q-03 | Exporter output format                                                           | JSON vs MessagePack/CBOR vs protobuf                                   | JSON for debuggability with optional compressed form; revisit if size matters                            | M0         |
| Q-04 | Z3 via bindings vs process                                                       | Rust `z3` crate vs SMT-LIB over a pipe                                 | Bindings behind the trait; fallback to SMT-LIB process for sandboxing                                    | M2         |
| Q-05 | How to represent `T*` that is sometimes owner, sometimes borrow (flow-dependent) | Split into two Rust places vs enum vs T2                               | Place splitting when flow-separable; else T2                                                             | M2         |
| Q-06 | Default for `assume_malloc_succeeds`                                             | false vs true                                                          | false (honest); document the performance and readability cost                                            | M2         |
| Q-07 | Hash-map iteration order in tests                                                | Deterministic hasher in tests vs mark tests unordered                  | Both: deterministic in tests, and flag order-dependent output                                            | M6         |
| Q-08 | Licensing model of the tool and `recast-rt`                                      | Permissive runtime + commercial core vs fully open vs source-available | Runtime crate permissively licensed (generated code depends on it); core decision for the business owner | Before M9  |
| Q-09 | Handling `char` signedness across targets                                        | Always `u8` vs per-target                                              | Per-target with recorded setting; `u8` when proven irrelevant                                            | M1         |
| Q-10 | Should T2 default to `Rc`/`RefCell` or arena handles for graph data              | Rc vs arena                                                            | Arena when closed world proven; otherwise `Rc` + `Weak`                                                  | M2         |
| Q-11 | Support for C++ coroutines                                                       | Never vs later                                                         | Flag; revisit after M7                                                                                   | M8         |
| Q-12 | Reporting overhead without the project's benchmarks                              | Skip vs synthetic microbenchmarks                                      | Synthetic around profiled hot functions, labeled as such                                                 | M8         |
| Q-13 | Cross-compilation support level in 1.0                                           | Native only vs listed targets                                          | Native plus x86_64/aarch64 Linux and Windows via listed targets                                          | M5         |
| Q-14 | How to treat `volatile`                                                          | Flag always vs map to `read_volatile`/`write_volatile`                 | Map simple `volatile` reads/writes in T3 with SAFETY note; flag MMIO patterns                            | M10+       |
| Q-15 | Output when translation is partially flagged: one crate with stubs vs split      | Single crate with `ffi` group                                          | Single crate (document 06 section 16.1)                                                                  | M5         |

---

## 4. Assumptions (explicit)

| ID   | Assumption                                                                 | If false                                                                              |
| ---- | -------------------------------------------------------------------------- | ------------------------------------------------------------------------------------- |
| A-01 | The input code compiles with Clang under the given `compile_commands.json` | Input error (exit 3); recast does not repair builds                                   |
| A-02 | Most real C/C++ code has dominant, simple ownership patterns               | R-01 materializes; tiers cover it with lower safety/performance                       |
| A-03 | Customers value verified evidence over raw translation percentage          | Positioning shifts toward speed and automation; verification becomes optional profile |
| A-04 | A rule-based core can reach useful T0 share without ML                     | Increase contracts and oracle loop; consider AI proposer earlier                      |
| A-05 | Existing test suites exist for at least part of target code                | Rely more on characterization and fuzzing; weaker evidence disclosed                  |
| A-06 | Rust stable remains backward compatible for generated constructs           | Pin toolchain; test on new releases                                                   |
| A-07 | Customers accept C ABI boundaries during migration                         | Add richer interop sooner                                                             |

---

## 5. Out of scope (explicit non-promises)

- Proving the original program correct.
- Automatic translation of code relying on unspecified compiler behavior without findings.
- Translating assembly, hardware-register code, or GPU kernels.
- Producing identical performance.
- Guaranteeing zero `unsafe` on arbitrary input (the goal is zero outside flagged and boundary code, reported honestly).
- Certifying compliance with any standard.
