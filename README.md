# recast

**Verified C/C++ to Rust translation.** Ownership-aware, deterministic, and honest about what it proved.

## Status

**M1 complete.** The M0 slice is tagged `v1.0.1` to `v1.0.11`; the M1 build points are `v1.1.0` to `v1.1.10`, `v1.1.11` records the exit criteria and `v1.1.12` the reference pass. The audit repairs are `v1.1.13` to `v1.1.28`, one tag per defect. [`docs/audits/m0.md`](docs/audits/m0.md) and [`docs/audits/m1.md`](docs/audits/m1.md) record the checks and remaining limits. `ci/m0-exit.sh` checks the original vertical slice, and `ci/m1-exit.sh` runs the L0 corpus: 10 cases, 174 differential calls, three 10^6-iteration property sweeps and operator mutation, all passing. **M2 is in progress**, one tag per step: `v1.2.0` opens the series, `v1.2.1` translates a pointer to a single object as a reference, `v1.2.2` owns and observes an output parameter in the V2 driver, `v1.2.3` adds the pointer corpus and `ci/m2-corpus.sh`, `v1.2.4` adds `ci/m2-refusals.sh`, and `v1.2.5` records the step in [`docs/audits/m2.md`](docs/audits/m2.md) with the assumption behind it and the remaining work. The specification documents define the full system: a Clang-based exporter, structured TIR, ownership and memory-safety analysis, tiered translation, a verification ladder, a safety gate and incremental C-ABI migration.

M1 translates scalars, control flow, structs by value, fixed arrays, string literals, enums and constants to safe Rust at T0, and verifies each corpus case against its C original. Heap ownership, macros, interop and the safety gate start at M2 and later milestones.

## What works now

- **Exporter** (`exporter/`): Clang LibTooling reads `compile_commands.json` and writes `recast-ast/1`, including target facts and typed function bodies for the L0 subset.
- **Frontend** (`recast-frontend`): lowers the export to TIR with interned types, stable item and node identifiers, explicit implicit casts and a canonical dump.
- **Validator** (`recast-ir`): enforces the M0 invariants from document 04 and reports every violation with a path.
- **Analysis** (`recast-analysis`): derives a CFG, runs liveness and definite-assignment checks, and reports the M1 undefined-behavior rules (`UB-001`, `UB-005`, `UB-007`, `UB-008`).
- **Translation** (`recast-translate`): emits Rust through a typed syntax tree, wrapping unsigned arithmetic explicitly and turning a pointer plus `size_t` pair into a slice only when the pointer is indexed and the length is only a length.
- **Verification** (`recast-verify`): V0 compilation, V2 differential execution against the C original, deterministic property sweeps and operator mutation testing.
- **CLI** (`recast`): `init`, `export`, `analyze`, `translate`, `verify`, `version` and `doctor`, with the documented exit codes.
- **Tests** (`recast-testkit`, `xtask`): snippet harness, golden runner, `xtask bless`, and CI that builds the generated crate and compares two runs byte for byte.

## Example

```c
int sum(const int *a, size_t n) {
    int s = 0;
    for (size_t i = 0; i < n; i++) s += a[i];
    return s;
}
```

becomes:

```rust
#![forbid(unsafe_code)]
#![allow(dead_code, non_camel_case_types, non_snake_case, unused_labels)]

/// recast: translated from input.c:3
pub fn sum(a: &[i32]) -> i32 {
    let mut s: i32 = 0;
    let mut i: usize = 0;
    while i < a.len() {
        s += a[i];
        i = i.wrapping_add(1);
    }
    s
}
```

## Quick start

```sh
cargo test --workspace
cargo run -p xtask -- check-goldens
cargo run -p recast-cli -- version
```

Build the exporter once (requirements and flags in [`exporter/README.md`](exporter/README.md)):

```sh
cmake -S exporter -B exporter/build -G Ninja -DCMAKE_BUILD_TYPE=Release
cmake --build exporter/build

cargo run -p recast-cli -- init
cargo run -p recast-cli -- export --compdb build/compile_commands.json \
  --exporter exporter/build/recast-exporter
cargo run -p recast-cli -- analyze
cargo run -p recast-cli -- translate
cargo run -p recast-cli -- verify --case corpus/l0/slices/case.json \
  --source corpus/l0/slices/source.c
```

`recast translate` writes a compiling crate to `recast-out/rust/`. `recast doctor` names every tool it checks and the fix for a missing one.

## Repository layout

| Path                | Purpose                                                      |
| ------------------- | ------------------------------------------------------------ |
| `docs/`             | Specification, architecture, guides, project records and audits |
| `crates/`           | Rust workspace: schema, IR, frontend, analysis, translate, verify, CLI, testkit |
| `exporter/`         | C++ Clang LibTooling exporter                                |
| `ci/`               | Fast pipeline, M0/M1 exit checks and documentation checks    |
| `tests/golden/`     | C input, exported AST and expected Rust                      |
| `tests/fixtures/`   | Shared C fixtures and their exported AST                     |
| `corpus/l0/`        | Committed L0 evaluation programs and case descriptions       |
| `models/`           | Library models for libc, POSIX and C++, from M2              |
| `benchmarks/`       | Overhead measurement protocol and manifests                  |
| `xtask/`            | Workspace automation and golden blessing                     |

## Documentation

Start with [`docs/README.md`](docs/README.md) for the index, conventions and status. Implementation work should then read [`docs/guides/implementation.md`](docs/guides/implementation.md). The critical review is [`docs/project/review-v1.md`](docs/project/review-v1.md) and the repository layout is [`docs/architecture/repository-layout.md`](docs/architecture/repository-layout.md).

## Contributing

See [`CONTRIBUTING.md`](CONTRIBUTING.md). Work on `dev`, use Conventional Commits, tag annotated only at verified states, and never commit directly to `main`. A wrong translation that passes the gate is the most serious kind of bug.

## License

Apache-2.0. See [`LICENSE`](LICENSE).
