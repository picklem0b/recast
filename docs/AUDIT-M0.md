# M0 Audit

Audit of the M0 skeleton at `v1.0.10`, performed 2026-10-25. The repairs below are tagged `v1.0.11`.

## Scope

- The export, frontend, TIR validation, translation, CLI and testkit paths.
- The three exit criteria in [`11-roadmap-and-milestones.md`](11-roadmap-and-milestones.md).
- Failure behavior, not only the two happy-path fixtures.
- Tags, remote state and the fast CI pipeline.

## Findings and repairs

| Class | Finding                                                                                                                                  | Repair                                                                                           |
| ----- | ---------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------ |
| P1    | A C parameter assigned in the body was emitted as a Rust parameter without `mut`, so `fact`-style functions did not compile.             | The translator collects assigned names and emits `mut` for those parameters.                     |
| P1    | A comparison used as an integer (`int y = x < 3;`, `(x < 3) + 1`, logical `&&` returned as `int`) rendered a Rust `bool` in an `i32` slot. | Type-directed translation inserts the C integer conversion (`(x < 3) as i32`) at value positions. |
| P2    | `--quiet` was accepted but human summaries still printed.                                                                                | Human summaries are suppressed; JSON output is never suppressed.                                 |
| P2    | The generated crate did not set `overflow-checks` for release builds, so signed overflow changed behavior between debug and release.      | Generated `Cargo.toml` sets `overflow-checks = true` in the release profile.                     |
| P3    | Locals were always emitted as `let mut`, producing warnings for immutable bindings.                                                       | Only locals and parameters that are actually assigned receive `mut`.                             |

The audit also ran a temporary differential fixture (`if`, `while`, calls, casts, compound assignment, prefix/postfix mutation, comparison-as-value) that passed C versus Rust at runtime across `add`, `twice`, `abs_val`, `fact`, `low`, `sign`, `less`, `both`, `nested`, `choose` and `even_sum`. The committed regression fixture `tests/fixtures/integer.c` keeps the two repaired cases.

## Verification after repair

| Check                                    | Result                                                                 |
| ---------------------------------------- | ---------------------------------------------------------------------- |
| `cargo test --workspace`                 | pass                                                                   |
| `cargo clippy --workspace --all-targets -- -D warnings` | pass                                                    |
| `rustfmt --check` on sources and generated crate | pass                                                           |
| `cargo run -p xtask -- check-goldens`    | 3 of 3                                                                 |
| `bash ci/fast.sh`                        | pass, two runs byte-identical                                          |
| `bash ci/m0-exit.sh`                     | all three exit criteria pass; C and Rust print `5 10 65`               |
| CLI integration tests                    | 6 pass, including quiet mode and release profile                       |
| Temporary runtime differential fixture   | C and Rust agree                                                        |

## Remaining limitations (deliberately deferred)

- `break` and `continue` are not in `recast-ast/1`; the exporter rejects them. A future `for` lowering must run the step on `continue` as document 06 requires.
- A non-void function whose final statement is an `if`/`else` where both branches return is refused. It is explicit failure, not incorrect output.
- `switch`, `goto`, records, macros, heap ownership and function pointers remain outside M0.
- Two `static` functions with the same name in different translation units are not yet representable; M0 assumes exported names are unique.
- Golden cases carry an exported `ast.json`; CI does not regenerate it from `input.c`, so a golden can drift from its C source until the exporter runs in the golden path.
- No analysis or verification crate is implemented at M0, so no safety claim beyond "compiles and matches the fixtures" is made.

## Release state

- `v1.0.0` approved structure; `v1.0.1` to `v1.0.9` build points; `v1.0.10` exit criteria.
- `v1.0.11` is the audit repair on top of those. Tags are annotated and published; `v1.0.10` is not moved.
