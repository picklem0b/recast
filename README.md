# recast

A deterministic C/C++ to Rust translator with a built-in verification system.

**Status: M0 implementation in progress.** The foundation specification and the v1.0 structure are in place; `v1.0.1` to `v1.0.9` are the nine M0 builds. The sixteen foundation documents define the intended system: a Clang-based frontend, a structured TIR, ownership and memory-safety analyses, tiered translation, a verification ladder, a safety gate and incremental C-ABI migration.

Start with [`docs/00-README.md`](docs/00-README.md) for the index, conventions and status. The independent critical review is in [`docs/REVIEW.md`](docs/REVIEW.md). The v1.0 structure and first-file plan are in [`docs/SCAFFOLD.md`](docs/SCAFFOLD.md).

## Repository layout

| Path                | Purpose                                                    |
| ------------------- | ---------------------------------------------------------- |
| `docs/`             | The foundation specification, review and scaffold          |
| `docs/00-README.md` | Index, requirements language and technology choices        |
| `docs/REVIEW.md`    | Objections, alternatives and decisions to settle before M0 |
| `docs/SCAFFOLD.md`  | v1.0 structure, crate ownership and first files            |
| `crates/`           | Planned Rust crates; no Cargo manifest yet                 |
| `exporter/`         | Planned C++/Clang LibTooling exporter                      |
| `models/`           | Planned library models for libc, POSIX and C++             |
| `corpus/`           | Pinned external projects, fetched not committed            |
| `benchmarks/`       | Overhead measurement protocol and manifests                |
| `tests/`            | Cross-crate golden tests                                   |
| `xtask/`            | Workspace automation                                       |

## Contributing

See [`CONTRIBUTING.md`](CONTRIBUTING.md). In short: work on `dev` or a branch cut from it, use Conventional Commits, create annotated tags only at verified states, and never commit directly to `main`.

## License

Apache-2.0. See [`LICENSE`](LICENSE).
