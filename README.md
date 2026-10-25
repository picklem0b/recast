# recast

A deterministic C/C++ to Rust translator with a built-in verification system.

**Status: specification only.** No implementation exists yet. The sixteen
foundation documents define the intended system: a Clang-based frontend, a
structured TIR, ownership and memory-safety analyses, tiered translation, a
verification ladder, a safety gate and incremental C-ABI migration.

Start with [`docs/00-README.md`](docs/00-README.md) for the index, conventions
and status. The independent critical review is in
[`docs/REVIEW.md`](docs/REVIEW.md).

## Repository layout

| Path | Purpose |
| --- | --- |
| `docs/` | The foundation specification and the review |
| `docs/00-README.md` | Index, requirements language and technology choices |
| `docs/REVIEW.md` | Objections, alternatives and decisions to settle before M0 |

## Contributing

See [`CONTRIBUTING.md`](CONTRIBUTING.md). In short: work on `dev` or a branch
cut from it, use Conventional Commits, create annotated tags only at verified
states, and never commit directly to `main`.

## Licence

Apache-2.0. See [`LICENSE`](LICENSE).
