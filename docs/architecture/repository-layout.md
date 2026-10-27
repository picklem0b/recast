# Repository layout

Status: M1. The workspace, the L0 pipeline, the analysis crate and the V0 to V2 verification ladder exist. Planned crates that have no implementation yet contain a library stub.

## Tree

```
recast/
  Cargo.toml                    # workspace manifest and shared dependency versions
  rust-toolchain.toml           # pinned compiler, rustfmt and clippy
  rustfmt.toml
  clippy.toml
  README.md
  CONTRIBUTING.md
  crates/
    recast-schema/              # recast-ast/1 types and validation
      src/lib.rs
      tests/round_trip.rs
    recast-ir/                  # TIR types, IDs, dump and validator
      src/{lib,id,types,program,validate,dump}.rs
      tests/validation.rs
    recast-frontend/            # recast-ast/1 to TIR
      src/lib.rs
      tests/lowering.rs
    recast-analysis/            # CFG, liveness, definite assignment, UB findings
      src/{lib,cfg,liveness,definite,ub}.rs
      tests/analysis.rs
    recast-translate/           # TIR to Rust
      src/{lib,rust}.rs
      tests/translation.rs
    recast-rt/                  # runtime shipped with generated code
    recast-verify/              # V0 to V2, differential drivers, mutation testing
      src/{lib,driver,mutation}.rs
    recast-interop/             # shims, layout checks and build integration
    recast-orchestrator/        # pipeline driver and cache
    recast-cli/                 # command line
      src/main.rs
      tests/cli.rs
    recast-testkit/             # snippet harness and golden runner
  exporter/                     # C++ Clang LibTooling exporter
    CMakeLists.txt
    src/main.cpp
    README.md
  xtask/                        # workspace automation; `cargo run -p xtask -- ...`
  ci/
    fast.sh                     # no-Clang build, determinism and generated-crate check
    m0-exit.sh                  # M0 exit criteria through the real exporter
    m1-exit.sh                  # M1 corpus, property and mutation criteria
    m2-corpus.sh                # M2 sub-corpus progress, not the M2 exit criteria
    m2-refusals.sh              # the constructs outside the current M2 step are refused
    check-docs.py               # Markdown link checker
  tests/                        # workspace-level test data, not a Cargo target
    fixtures/                   # committed C snippets and their exported ASTs
    golden/                     # expected translation output per case
  corpus/
    l0/                         # committed L0 teaching programs and case descriptions
    m2/                         # committed M2 programs, one directory per build point
  models/                       # library models (populated from M2)
  benchmarks/                   # benchmark protocol and manifests
  docs/
    README.md                   # index, conventions and status
    architecture/               # system structure and specifications
    guides/                     # task-oriented instructions
    reference/                  # glossary and worked examples
    project/                    # vision, roadmap, decisions, research, review
    audits/                     # point-in-time audit reports
```

## Why the tree has this shape

Cargo defines the Rust package layout. A package has its manifest, its library or binary sources under `src/`, and its integration tests under `tests/`. A package cannot be moved wholesale under a repository-level `src/` without breaking `cargo` discovery, and a repository-level `src/` would add a second layout that every tool and contributor must learn. The workspace therefore keeps packages in `crates/`, each with the standard local `src/` and `tests/`.

The top-level `tests/` is not a Cargo target because the workspace root is not a package. It holds data shared by several packages: exported fixtures and translation goldens. Cargo integration tests always live in `crates/<crate>/tests/`. The split is:

| Kind of test or data                          | Location                                      |
| --------------------------------------------- | --------------------------------------------- |
| Unit test for a private function              | `#[cfg(test)]` in the same source file        |
| Integration test of one crate                 | `crates/<crate>/tests/`                       |
| Golden translation case                       | `tests/golden/<milestone>/<case>/`            |
| C source plus exported AST used by several crates | `tests/fixtures/`                         |
| Committed evaluation program                  | `corpus/l0/<case>/`                           |
| Test toolchain wiring                         | `ci/*.sh` or `xtask`                          |

`tests/` is the established Rust name for test material and is unambiguous next to `crates/<crate>/tests/` once the rule above is stated. Renaming it to `testdata/` would separate it from Cargo vocabulary without changing its role; the current name is kept because every path in exported fixtures, golden files and documentation already uses it.

## Package ownership

| Package               | Owns                                                                 | Implemented files                                              |
| --------------------- | -------------------------------------------------------------------- | -------------------------------------------------------------- |
| `recast-schema`       | `recast-ast/1` types, serialization and schema version               | `src/lib.rs`                                                   |
| `recast-ir`           | TIR types, IDs, program structure, validation and dump format        | `src/{id,types,program,validate,dump}.rs`                      |
| `recast-frontend`     | Schema to TIR lowering and translation-unit merge                    | `src/lib.rs`                                                   |
| `recast-analysis`     | CFG, liveness, definite assignment and UB checks for the M1 subset   | `src/{cfg,liveness,definite,ub}.rs`                            |
| `recast-translate`    | Type and expression mapping, statement emission, Rust printing       | `src/{lib,rust}.rs`                                            |
| `recast-rt`           | Runtime support for generated code: bytes, arena, checked, FFI, raw  | `src/lib.rs` (stub)                                            |
| `recast-verify`       | V0 to V2 drivers, differential runs, property sweeps, mutation tests | `src/{lib,driver,mutation}.rs`                                 |
| `recast-interop`      | Shim generation, layout assertions and build-system integration      | `src/lib.rs` (stub)                                            |
| `recast-orchestrator` | Pipeline driver, scheduler and content cache                        | `src/lib.rs` (stub)                                            |
| `recast-cli`          | Command parsing, output and exit codes                               | `src/main.rs`                                                  |
| `recast-testkit`      | Snippet harness and golden runner                                    | `src/{lib,golden,harness}.rs`                                  |
| `exporter`            | Clang AST export, macro records and target layout                    | `src/main.cpp`                                                 |
| `xtask`               | Workspace automation and golden blessing                             | `src/{main,lib}.rs`                                            |

## Dependency direction

The Cargo graph is the acyclic graph in [`overview.md`](overview.md) section 4. The list below is the stage order used to read the tree, not a blanket permission to add an edge. Every dependency must appear in that section; if a manifest needs an edge not shown there, record an ADR before adding it.

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

`recast-rt` is standalone and is depended on by generated code, not by the tool. `recast-testkit` is a development dependency only.

## Placement rules

- A new pipeline stage module belongs in the owning package under `crates/<package>/src/`, named for what it defines.
- A test that exercises a package's public interface belongs in that package's `tests/` directory. A test that needs a golden case or a shared fixture uses `tests/`.
- A new corpus program is self-contained under `corpus/l0/<case>/` with `source.c` and `case.json`. Corpus programs are committed because they are evaluation inputs, not dependencies.
- External projects used for scale tests are fetched into a cache and never committed.
- A document belongs under the `docs/` section that matches its purpose, with an entry in `docs/README.md`.
- Repository-wide automation belongs in `xtask` when it is Rust or needs the workspace model, and in `ci/` when it only wires external tools. A shell script is not a home for logic that has a natural Rust implementation.

## Conventions

### Naming

Package directories use `kebab-case` with the `recast-` prefix. Rust modules and source files use `snake_case`. A file is named for the thing it defines, not for the layer it belongs to. Prefer `foo.rs` plus `foo/` over `mod.rs` for nested modules.

### Tests

Unit tests live in the file they test under `#[cfg(test)]`. Integration tests live in `crates/<crate>/tests/`, following Cargo's package layout. Workspace-level data lives in `tests/`: translation goldens at `tests/golden/<milestone>/<case>/` with `input.c` or `input.cpp`, `config.toml`, `expected.rs` and `expected.explain.json`, and exported fixtures under `tests/fixtures/`. The top-level `tests/` directory is data, not a Cargo test target.

### Documentation

Documentation is organized by purpose under `docs/`: `architecture/` for system structure, `guides/` for task-oriented instructions, `reference/` for lookup material, `project/` for planning and decisions, and `audits/` for point-in-time reports. `docs/README.md` is the index; every document is linked from it. Document numbers `01` to `15` are stable identifiers used by rule tables and cross-references, not filenames. A new document is added under the matching section and linked from the index; it takes the next free number or none.

Markdown uses one sentence or clause per line, padded tables and US spelling. Documentation states requirements and facts in the third person; it does not address a reader as an individual and does not record chat context. `README.md` is the index name used at every directory level because GitHub renders it automatically. `INDEX.md` is reserved for static-site generators and is not used.

### Code comments

Comments explain why, not what. Do not add decorative separator banners. Public Rust items get `///` documentation. Every `unsafe` block gets a `// SAFETY:` comment stating the invariant.

### Formatting

Rust uses `rustfmt` with the pinned configuration; C++ uses `clang-format`. No JavaScript or TypeScript formatter is configured because the repository contains neither language.

### Errors and findings

Pipeline stages return `Result<Output, StageError>` plus a stream of `Finding` values. Panics indicate a recast bug and are caught at the item boundary. Test code may panic; production pipeline code does not.

### Determinism

Use ordered collections or sort by a stable key before output. Do not read the clock, environment or network in a stage that affects generated bytes. Seeds and budgets are recorded.

### Identifiers

Rules use the prefixes in [`translation.md`](translation.md) section 19: `TR-0xx` types, `TR-1xx` items, `TR-2xx` pointers, `TR-3xx` control flow, `TR-4xx` expressions, `TR-5xx` library mapping, `TR-6xx` preprocessor, `IDM-nnn` idioms. Analysis rules use `OWN-`, `NUL-`, `ARR-` and `UB-`.

### Schema versions

The schema names are `recast-ast/1`, `tir/1` and `recast-report/1`. A breaking change bumps the integer and the reader refuses an unknown major version.

### Licenses

The repository is Apache-2.0. Do not add per-file license headers unless a dependency or policy requires them.

### Versions and tags

`spec-v1.0.x` tags the specification set. `v1.0.x` tags implementation states. Tags are annotated and their messages start with the tag name. A tag is created only after its exit criteria pass, and it publishes exactly one commit — a build point, a repair or a document state. Nothing is batched into a tag: a step that is worth a commit is worth its own tag.

### Commits

Follow [`CONTRIBUTING.md`](../../CONTRIBUTING.md): Conventional Commits, one purpose per commit, a body that explains why, and no attribution footers.

## Version history

| Tag                  | State                                                                    |
| -------------------- | ------------------------------------------------------------------------ |
| `spec-v1.0.x`        | Specification set.                                                       |
| `v1.0.0`             | Approved structure and package ownership.                                |
| `v1.0.1` to `v1.0.9` | M0 build points: exporter, frontend, TIR, translation, CLI and CI.       |
| `v1.0.10`            | M0 exit criteria.                                                        |
| `v1.0.11`            | M0 audit repairs.                                                        |
| `v1.1.0` to `v1.1.5` | M1 build points: schema, IR, exporter, frontend, analysis, translation.  |
| `v1.1.6` to `v1.1.10`| M1 build points: verification, CLI, corpus, CI and documents.            |
| `v1.1.11`            | M1 exit criteria.                                                        |
| `v1.1.12`            | M1 references point at the page tags.                                    |
| `v1.1.13`            | M1 audit repairs.                                                        |
| `v1.1.14`            | M1 repair note: the signed-`char` check.                                 |
| `v1.1.15`            | M1 repair: literal typing.                                               |
| `v1.1.16`            | M1 repair: a constant cast folds.                                        |
| `v1.1.17`            | M1 repair: a void cast discards.                                         |
| `v1.1.18`            | M1 repair: a cast before a comparison.                                   |
| `v1.1.19`            | M1 repair: unsigned constants.                                           |
| `v1.1.20`            | M1 repair: every statement of a switch arm.                              |
| `v1.1.21`            | M1 repair: `continue` leaves the switch.                                 |
| `v1.1.22`            | M1 repair: typedef record names.                                         |
| `v1.1.23`            | M1 repair: array arguments in the V2 driver.                             |
| `v1.1.24`            | M1 repair: index bases.                                                  |
| `v1.1.25`            | M1 repair: compound shifts.                                              |
| `v1.1.26`            | M1 repair: typedef initializers.                                         |
| `v1.1.27`            | M1 repair: typedef spellings in the V2 driver.                           |
| `v1.1.28`            | M1 repair: style of the repair set.                                      |
| `v1.1.29`            | M1 audit: the third pass recorded.                                       |
| `v1.2.0`             | M2 opens: the workspace version is 1.2.0.                                |
| `v1.2.1`             | M2: a pointer to one object is a reference.                              |
| `v1.2.2`             | M2: the V2 driver owns and observes a pointer argument.                  |
| `v1.2.3`             | M2: the pointer corpus and `ci/m2-corpus.sh`.                            |
| `v1.2.4`             | M2: `ci/m2-refusals.sh` checks the refusal list.                         |
| `v1.2.5`             | M2: the audit records the pointer step.                                  |
| `v1.1.30`            | M1 audit: the tag lists include `v1.1.29`.                               |
