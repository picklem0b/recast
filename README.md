# recast

**Verified C/C++ to Rust translation.** Ownership-aware, deterministic, and honest about what it proved.

## Status

**M0 complete.** The nine build points are tagged `v1.0.1` to `v1.0.9`, `v1.0.10` records the three exit criteria, and `v1.0.11` carries the audit repairs. The audit and its remaining limits are in [`docs/AUDIT-M0.md`](docs/AUDIT-M0.md). `ci/m0-exit.sh` checks them: `add` and `sum` cross C to compiling Rust and agree with the C at runtime, two complete runs are byte-identical, and the exported schema round-trips. The sixteen foundation documents define the full system: a Clang-based exporter, structured TIR, ownership and memory-safety analysis, tiered translation, a verification ladder, a safety gate and incremental C-ABI migration.

M0 proves the vertical slice, not the product. It translates scalar functions and the `const int *, size_t` slice pattern to safe Rust that compiles. Records, macros, heap ownership and the verification ladder start at M1 and M2.

## What works now

- **Exporter** (`exporter/`): Clang LibTooling reads `compile_commands.json` and writes `recast-ast/1`, including target facts and typed function bodies for the L0 subset.
- **Frontend** (`recast-frontend`): lowers the export to TIR with interned types, stable item and node identifiers, explicit implicit casts and a canonical dump.
- **Validator** (`recast-ir`): enforces the M0 invariants from document 04 and reports every violation with a path.
- **Translation** (`recast-translate`): emits Rust through a typed syntax tree, wrapping unsigned arithmetic explicitly and turning a pointer plus `size_t` pair into a slice only when the pointer is indexed and the length is only a length.
- **CLI** (`recast`): `init`, `export`, `analyze`, `translate`, `version` and `doctor`, with the documented exit codes.
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

/// recast: translated from input.c:3
pub fn sum(a: &[i32]) -> i32 {
    let mut s: i32 = 0;
    let mut i: usize = 0;
    while i < a.len() {
        s += a[i];
        i += 1;
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
```

`recast translate` writes a compiling crate to `recast-out/rust/`. `recast doctor` names every tool it checks and the fix for a missing one.

## Repository layout

| Path                | Purpose                                                      |
| ------------------- | ------------------------------------------------------------ |
| `docs/`             | Foundation specification, review and scaffold                |
| `crates/`           | Rust workspace: schema, IR, frontend, translate, CLI, testkit |
| `exporter/`         | C++ Clang LibTooling exporter                                |
| `ci/`               | Fast pipeline, exporter pipeline and documentation checks    |
| `tests/golden/`     | C input, exported AST and expected Rust                      |
| `tests/fixtures/`   | C fixtures and their exported AST                            |
| `models/`           | Library models for libc, POSIX and C++, from M2              |
| `corpus/`           | Pinned external projects, fetched not committed              |
| `benchmarks/`       | Overhead measurement protocol and manifests                  |
| `xtask/`            | Workspace automation and golden blessing                     |

## Documentation

Start with [`docs/00-README.md`](docs/00-README.md) (index, conventions, status). If you are building recast, read [`docs/15-builder-brief.md`](docs/15-builder-brief.md) next. The critical review is [`docs/REVIEW.md`](docs/REVIEW.md) and the v1.0 structure is [`docs/SCAFFOLD.md`](docs/SCAFFOLD.md).

## Contributing

See [`CONTRIBUTING.md`](CONTRIBUTING.md). Work on `dev`, use Conventional Commits, tag annotated only at verified states, and never commit directly to `main`. A wrong translation that passes the gate is the most serious kind of bug.

## License

Apache-2.0. See [`LICENSE`](LICENSE).
