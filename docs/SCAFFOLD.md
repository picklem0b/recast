# v1.0 Scaffold

Status: structure proposal for `v1.0.0-rc.1`. The v1.0.x line contains conventions, directory boundaries and the first-file plan only. No Rust or C++ implementation is part of it. The v1.1.x line starts only after explicit approval.

## Scope

The scaffold answers three questions:

1. Where does each kind of file live?
2. Which crate owns each responsibility?
3. Which files are created first when implementation starts?

It does not create a Cargo workspace, compile anything, add dependencies or pin a toolchain version. Those are v1.1.x decisions.

## Repository tree

```
recast/
  Cargo.toml                    # v1.1
  rust-toolchain.toml           # v1.1
  exporter/                     # C++/Clang LibTooling exporter
    CMakeLists.txt              # v1.1
    src/
  crates/
    recast-schema/
    recast-ir/
    recast-frontend/
    recast-analysis/
    recast-translate/
    recast-rt/
    recast-verify/
    recast-interop/
    recast-orchestrator/
    recast-cli/
    recast-testkit/
  models/                       # library models for libc, POSIX and the C++ standard library
  corpus/                       # pinned external projects, fetched not committed
  benchmarks/                   # benchmark protocol and manifests
  tests/
    golden/                     # one directory per rule, then one per case
  xtask/                        # workspace automation
  docs/
```

## Crate responsibilities and first files

The files below are the first ones to create in v1.1.x, in this order within each crate. They are not created by the scaffold.

| Crate                 | Owns                                                                                          | First files                                                                                              |
| --------------------- | --------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------- |
| `recast-schema`       | `recast-ast/1` types, serialization and schema version                                        | `src/lib.rs`, `src/ast.rs`, `tests/roundtrip.rs`                                                         |
| `recast-ir`           | TIR types, IDs, program structure, validation and dump format                                 | `src/lib.rs`, `src/id.rs`, `src/program.rs`, `src/types.rs`, `src/validate.rs`                           |
| `recast-frontend`     | Schema to TIR lowering, translation-unit merge, configurations and macros                     | `src/lib.rs`, `src/lower.rs`, `src/merge.rs`, `src/config.rs`, `src/macros.rs`                           |
| `recast-analysis`     | Points-to, ownership, nullability, extent, lifetimes, escape, UB and summaries                | `src/lib.rs`, `src/points_to.rs`, `src/ownership.rs`, `src/nullability.rs`, `src/extent.rs`, `src/ub.rs` |
| `recast-translate`    | Tier selection, type and expression mapping, statement emission and the idiom pass            | `src/lib.rs`, `src/tier.rs`, `src/type_map.rs`, `src/emit.rs`, `src/idiom.rs`                            |
| `recast-rt`           | Runtime support shipped with generated code: bytes, arena, checked cells, FFI and raw helpers | `src/lib.rs`, `src/bytes.rs`, `src/arena.rs`, `src/checked.rs`, `src/ffi.rs`, `src/raw.rs`               |
| `recast-verify`       | Verification ladder, differential harness, fuzz glue, safety gate and report                  | `src/lib.rs`, `src/ladder.rs`, `src/differential.rs`, `src/gate.rs`, `src/report.rs`                     |
| `recast-interop`      | Shim generation, layout assertions and build-system integration                               | `src/lib.rs`, `src/shims.rs`, `src/layout.rs`, `src/build.rs`                                            |
| `recast-orchestrator` | Pipeline driver, scheduler, content cache and stage results                                   | `src/lib.rs`, `src/pipeline.rs`, `src/cache.rs`, `src/schedule.rs`                                       |
| `recast-cli`          | Command parsing, output and exit codes                                                        | `src/main.rs`, `src/commands.rs`, `src/commands/run.rs`                                                  |
| `recast-testkit`      | Snippet harness, golden runner and annotation parser                                          | `src/lib.rs`, `src/snippet.rs`, `src/golden.rs`                                                          |
| `exporter`            | Clang AST export, macro expansion records and target layout                                   | `CMakeLists.txt`, `src/main.cpp`, `src/ast_exporter.cpp`, `src/pp_records.cpp`                           |
| `xtask`               | Workspace automation and CI helpers                                                           | `src/main.rs`, `src/ci.rs`                                                                               |

## Dependency direction

The Cargo graph is the acyclic graph in `docs/03-architecture.md` section 4. The list below is the stage order used to read the tree; it is not a blanket permission to add an edge. Every dependency must appear in that section. If a manifest needs an edge not shown there, record an ADR before adding it.

```
recast-schema
recast-ir
recast-frontend
recast-analysis
recast-translate
recast-verify
recast-interop
recast-orchestrator
recast-cli
```

`recast-rt` is standalone and is depended on by generated code, not by the tool. `recast-testkit` is a development dependency only. If a manifest needs an edge outside this order, record an ADR before adding it.

## First vertical slice

The v1.1.x work starts with M0 only, in this order:

1. Root `Cargo.toml`, `rust-toolchain.toml` and workspace lints.
2. `recast-schema` and `recast-ir`, including the validator.
3. `exporter` for functions with integer parameters and arithmetic.
4. `recast-frontend` and `recast-translate` for the `checksum` example.
5. `recast-cli` with `version` and `translate`.
6. `recast-testkit` with the snippet harness and one golden test.
7. The fast CI pipeline and the determinism check.

Analysis, verification, interop and C++ support are later milestones. They do not get starter files until their milestone starts.

## Conventions

### Naming

Crate packages and directories use `kebab-case` with the `recast-` prefix. Rust modules and source files use `snake_case`. A file is named for the thing it defines, not for the layer it belongs to. Prefer `foo.rs` plus `foo/` over `mod.rs` for nested modules.

### Tests

Unit tests live in the file they test under `#[cfg(test)]`. Integration tests live in `crates/<crate>/tests/`. Translation goldens live at `tests/golden/<rule-id>/<case>/` with `input.c` or `input.cpp`, `config.toml`, `expected.rs` and `expected.explain.json`. Corpus sources are fetched into a cache and never committed.

### Documentation

The numbered foundation set `00` to `15` is frozen. New working documents use uppercase names at the repository root or under `docs/` and are linked from `README.md`; they do not renumber the foundation set. Rule and config identifiers in code must have a matching document entry.

### Code comments

Comments explain why, not what. Do not add decorative separator banners. Public Rust items get `///` documentation. Every `unsafe` block gets a `// SAFETY:` comment stating the invariant.

### Formatting

Rust uses `rustfmt` with a pinned configuration; C++ uses `clang-format`. Markdown follows the foundation documents: one line per paragraph, padded tables and US spelling. No JavaScript or TypeScript formatter is configured because the scaffold contains neither language.

### Errors and findings

Pipeline stages return `Result<Output, StageError>` plus a stream of `Finding` values. Panics indicate a recast bug and are caught at the item boundary. Test code may panic; production pipeline code does not.

### Determinism

Use ordered collections or sort by a stable key before output. Do not read the clock, environment or network in a stage that affects generated bytes. Seeds and budgets are recorded.

### Identifiers

Rules use the prefixes in `docs/06-translation-specification.md` section 19: `TR-0xx` types, `TR-1xx` items, `TR-2xx` pointers, `TR-3xx` control flow, `TR-4xx` expressions, `TR-5xx` library mapping, `TR-6xx` preprocessor, `IDM-nnn` idioms. Analysis rules use `OWN-`, `NUL-`, `ARR-` and `UB-`.

### Schema versions

The schema names are `recast-ast/1`, `tir/1` and `recast-report/1`. A breaking change bumps the integer and the reader refuses an unknown major version.

### Licenses

The repository is Apache-2.0. Do not add per-file license headers unless a dependency or policy requires them.

### Versions and tags

`spec-v1.0.x` tags the foundation specification. `v1.0.x` tags the structural scaffold; `v1.0.0` is reserved for the approved structure and does not imply implementation. `v1.1.x` tags implementation slices and starts only after explicit approval. Tags are annotated and their messages start with the tag name.

### Commits

Follow `CONTRIBUTING.md`: Conventional Commits, one purpose per commit, a body that explains why, and no attribution footers.

## What must be true before v1.0.0

- The tree and crate ownership are agreed.
- The first-file list is agreed.
- Every planned path has one owner and no cyclic dependency.
- The README links resolve.
- No implementation has entered the v1.0.x line.

## What waits for v1.1.x

The Cargo workspace, toolchain pin, dependencies, Rust and C++ source, CI workflows, test harnesses and the first vertical slice all wait for v1.1.x and explicit permission.
