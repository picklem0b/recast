# 15. Builder Brief

Instructions for the AI (or any engineer) building recast from this document set. Read this file second, right after `docs/README.md`.

---

## 1. Mission in one paragraph

Build **recast**: a deterministic translator from C/C++ to Rust that (1) understands memory ownership well enough to emit safe Rust wherever it can prove it, (2) falls back to clearly labeled runtime-checked, contained-unsafe, or preserved-original forms where it cannot, and (3) verifies every result against the original program and reports honestly what was and was not established. Trust is the product.

## 2. Reading order

1. `docs/README.md` (index, conventions)
2. `docs/guides/implementation.md` (this file)
3. `docs/project/vision-and-scope.md`, `docs/reference/glossary.md`
4. `docs/architecture/overview.md`, `docs/architecture/ir.md`
5. `docs/architecture/analysis.md`, `docs/architecture/translation.md`
6. `docs/architecture/verification.md`, `docs/guides/interop-and-builds.md`
7. `docs/guides/cli-and-configuration.md`, `docs/guides/testing-and-corpus.md`
8. `docs/project/roadmap.md` (what to build in what order)
9. `docs/project/decisions-and-risks.md`, `docs/project/research.md`, `docs/reference/worked-examples.md`

## 3. Operating rules (non-negotiable)

1. **Correctness over coverage.** When unsure, flag (T4); never guess a translation.
2. **Never claim more than the evidence.** Use the evidence classes and approved wording (document 07 section 5).
3. **Determinism.** Same inputs and config, same bytes out. Sorted containers, seeded solvers, no timestamps in outputs, parallelism that cannot change results.
4. **No AI in the core path** (ADR-003). Do not add model calls to translation or verification.
5. **Do not trust analysis in the verifier** (ADR-010).
6. **Conservative defaults** (ADR-006): may-alias, maybe-null, unknown ownership until proven.
7. **Respect scope.** Language levels (document 01) and the always-flagged list (document 06 section 16) bound the work. Do not silently expand them.
8. **Every rule has a test.** A rule ID without a golden test and a worked example does not exist.
9. **Vertical slices.** Get a function through every stage before deepening any stage.
10. **Record decisions.** When a document is ambiguous, pick the recommended default (document 12 section 3), note it in an ADR, continue.

## 4. Build order (summary of document 11)

M0 skeleton and one function end to end, then M1 C scalars and control flow, M2 pointers and ownership, M3 verification and gate, M4 macros and configurations, M5 interop, M6 C++ core, M7 templates, M8 hardening, M9 release.
Do not begin a milestone until the previous exit criteria hold.

## 5. Technology choices (defaults)

| Area                 | Choice                                                             |
| -------------------- | ------------------------------------------------------------------ |
| Core language        | Rust (stable, pinned), workspace of crates (document 03 section 5) |
| Frontend exporter    | C++ with Clang LibTooling, emitting `recast-ast/1` JSON            |
| Solver               | Z3 behind `ConstraintSolver`                                       |
| Rust code generation | Internal Rust AST printed via `prettyplease`/`rustfmt`             |
| Serialization        | `serde` with canonical ordering                                    |
| Parallelism          | `rayon` with deterministic reduction                               |
| CLI                  | `clap`                                                             |
| Logging              | `tracing`                                                          |
| Tests                | `cargo nextest`, `insta`-style goldens (own runner), `proptest`    |
| Fuzzing              | libFuzzer/AFL++ (C++ side), `cargo-fuzz` (Rust)                    |
| Sandboxing           | Rootless containers                                                |
| Build integration    | Corrosion (CMake), `rules_rust` (Bazel)                            |

## 6. First week of work (concrete)

1. Create the workspace and CI per document 03 and 11 (M0).
2. Define `recast-ast/1` and the TIR types; write the validator for the ten invariants (document 04).
3. Write the exporter for functions with integer parameters and arithmetic.
4. Frontend: AST to TIR; dump format; golden tests for the dump.
5. Translator: emit `checksum` (example 1) and `sum` (example 2, using hand-set facts at first).
6. Add the snippet harness and the first behavioral differential test.
7. Make the determinism check run in CI.

## 7. Definition of done for any change

- Builds, lints and formatted.
- Unit and golden tests added or updated.
- Document references updated (rule IDs, config keys, finding codes).
- Determinism test passes.
- If it touches translation or analysis: corpus metrics not regressed; L7 soundness clean.
- No new `unsafe` in generated output without a SAFETY comment and test; no new `unsafe` in `recast-rt` outside `ffi`/`raw`.

## 8. When stuck

- **Ambiguous requirement:** choose the recommended default, write an ADR, continue.
- **Analysis cannot decide:** emit `Unknown`, move up a tier, add a finding; improve precision later.
- **Construct is hard:** flag it (T4), add it to the known-hard corpus with the expected flag, move on.
- **Verification is slow:** reduce budget or profile; never skip silently; list the skipped rung.
- **A failing test may be the test's fault:** prove that by reading the original semantics; fix the test with a note. Never loosen a comparison to make it pass.

## 9. Things that look tempting and are wrong

- Running the C preprocessor first and translating the output (loses macros and configurations).
- Mapping `std::string` straight to `String`.
- Replacing `T*` with `&mut T` because "it compiles".
- Letting the idiom pass change behavior.
- Using wall-clock time budgets in fuzzing for results that appear in the report.
- Claiming "safe Rust" for a crate containing T2 or T3 forms without qualification.
- Adding an LLM call to speed up ownership inference.
- Treating passing tests as proof.

## 10. Output the builder must produce

- The workspace and crates of document 03, building and tested.
- `recast` binary with commands of document 09.
- `recast-rt` runtime crate.
- Models (`models/libc.toml`, `models/posix.toml`, `models/cxx-std.toml`).
- Test corpus scripts and manifests (document 10).
- Reports conforming to `recast-report/1`.
- Updated documents and ADRs reflecting every decision made during the build.
