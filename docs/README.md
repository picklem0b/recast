# recast documentation

recast is a C/C++ to memory-safe Rust translator with a built-in verification system. It reads an existing C or C++ codebase, builds a semantic model of how the code uses memory, writes Rust that preserves the original behavior, and checks that claim with tests, fuzzing, sanitizers and, where feasible, formal tools. The output is a Rust crate plus a machine-readable verification report.

This directory contains the complete specification for building recast. The documents are self-contained: a reader with no other context can build the system from them.

---

## Document map

Document numbers are stable identifiers used by rule tables, code comments and cross-references. Filenames describe the subject and may change without renumbering a document.

| ID  | Document                                                             | Purpose                                                                                        |
| --- | -------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------- |
| 01  | [`project/vision-and-scope.md`](project/vision-and-scope.md)         | Problem, product definition, goals, non-goals, success metrics, supported language subset      |
| 02  | [`reference/glossary.md`](reference/glossary.md)                     | Precise definitions of every term used                                                         |
| 03  | [`architecture/overview.md`](architecture/overview.md)               | Components, data flow, repository layout, technology choices, cross-cutting rules              |
| 04  | [`architecture/ir.md`](architecture/ir.md)                           | The internal representation (TIR) that every stage reads and writes                            |
| 05  | [`architecture/analysis.md`](architecture/analysis.md)               | Alias, ownership, lifetime, nullability, array, UB, template and macro analyses                |
| 06  | [`architecture/translation.md`](architecture/translation.md)         | Exact C/C++ to Rust mapping rules, memory strategy tiers, runtime support crate, output layout |
| 07  | [`architecture/verification.md`](architecture/verification.md)       | Verification ladder, safety gate, report schema, determinism rules                             |
| 08  | [`guides/interop-and-builds.md`](guides/interop-and-builds.md)       | Mixed C++/Rust builds, FFI shims, incremental migration protocol                               |
| 09  | [`guides/cli-and-configuration.md`](guides/cli-and-configuration.md) | Commands, `recast.toml`, contracts file, exit codes, output directory layout                   |
| 10  | [`guides/testing-and-corpus.md`](guides/testing-and-corpus.md)       | Test pyramid, golden tests, corpus selection, benchmarks, CI                                   |
| 11  | [`project/roadmap.md`](project/roadmap.md)                           | Ordered milestones with acceptance criteria                                                    |
| 12  | [`project/decisions-and-risks.md`](project/decisions-and-risks.md)   | Risk register, decision log (ADRs), open questions                                             |
| 13  | [`project/research.md`](project/research.md)                         | Prior art, research findings, market and regulatory context, sources                           |
| 14  | [`reference/worked-examples.md`](reference/worked-examples.md)       | Concrete C/C++ inputs with expected Rust outputs and tiers                                     |
| 15  | [`guides/implementation.md`](guides/implementation.md)               | Operating instructions for an AI model or engineer doing the build                             |

Supporting documents:

| Document                                                                 | Purpose                                                     |
| ------------------------------------------------------------------------ | ----------------------------------------------------------- |
| [`architecture/repository-layout.md`](architecture/repository-layout.md) | Crate ownership, file naming, dependency direction, tags    |
| [`project/review-v1.md`](project/review-v1.md)                           | Critical review of the v1.0 specification                   |
| [`audits/m0.md`](audits/m0.md)                                           | M0 audit: defects found, repairs and remaining limitations  |
| [`audits/m1.md`](audits/m1.md)                                           | M1 audit: corpus, verification results and remaining limits |
| [`audits/m2.md`](audits/m2.md)                                           | M2 audit in progress: build points, assumption and remaining work |

---

## Reading order

The recommended order for a builder:

1. [`project/vision-and-scope.md`](project/vision-and-scope.md)
2. [`reference/glossary.md`](reference/glossary.md)
3. [`architecture/overview.md`](architecture/overview.md)
4. [`architecture/ir.md`](architecture/ir.md)
5. The specification for the stage being built: [`architecture/analysis.md`](architecture/analysis.md), [`architecture/translation.md`](architecture/translation.md), [`architecture/verification.md`](architecture/verification.md) or [`guides/interop-and-builds.md`](guides/interop-and-builds.md)
6. [`guides/implementation.md`](guides/implementation.md) for operating rules
7. [`project/roadmap.md`](project/roadmap.md) for build order

[`reference/worked-examples.md`](reference/worked-examples.md) is kept open alongside rule implementation.

---

## Conventions

### Requirement language

The words **MUST**, **MUST NOT**, **SHOULD**, **SHOULD NOT** and **MAY** are used as in RFC 2119.

- **MUST / MUST NOT**: required for correctness or for the product's trust claims. Violating it is a bug.
- **SHOULD / SHOULD NOT**: the default; deviate only with a recorded reason (see document 12).
- **MAY**: optional.

### Evidence labels

Every non-trivial claim in these documents carries one of three provenance labels:

- **[Research]**: stated by a published paper, project page or official source. Listed in document 13.
- **[Decision]**: a design choice made for recast. It is binding until changed through the decision log.
- **[Assumption]**: believed true but not verified. A builder SHOULD verify it early; if false, record the correction.

Where a section has no label, it is a [Decision].

### Status vocabulary

- **Specified**: the document defines the behavior fully enough to implement.
- **Outlined**: the shape is defined but details are left to the builder; the builder MUST record the chosen details as decisions.
- **Research-grade**: technique is known to be hard or unproven; implementation is optional for early milestones.

### Naming

- The product and CLI are `recast`. The internal representation is **TIR**. The runtime support crate is `recast_rt`.
- Crates and modules use `snake_case`. Rule identifiers use `UPPER-KEBAB` with a category prefix (for example `OWN-001`, `UB-014`, `TR-203`).

### Documentation layout

- `docs/README.md` is the index. Every document is linked from it.
- Section directories group documents by purpose: `architecture/` for system structure, `guides/` for task-oriented instructions, `reference/` for lookup material, `project/` for planning and decisions, and `audits/` for point-in-time reports.
- New documents are added under the section that matches their purpose and linked from this index. Document numbers remain stable; a new document takes the next free number or none.
- Directory indexes use `README.md`, the convention GitHub renders automatically. `INDEX.md` is reserved for static-site generators and is not used here.

---

## Document set status

| Area                    | Status                                                      |
| ----------------------- | ----------------------------------------------------------- |
| Vision, scope, glossary | Specified                                                   |
| Architecture, IR        | Specified                                                   |
| Analysis                | Specified for the baseline; refinements Outlined            |
| Translation rules       | Specified for Tiers 0 to 3; generic recovery Research-grade |
| Verification            | Specified for rungs V0 to V4; V5 Research-grade             |
| Interop                 | Specified for C ABI shims; deeper C++ interop Outlined      |
| CLI and configuration   | Specified                                                   |
| Testing and corpus      | Specified; C++ corpus selection Outlined                    |
| Roadmap                 | Specified                                                   |

Specification version: 1.0. M0 and M1 are implemented; implementation status is tracked in [`project/roadmap.md`](project/roadmap.md) and the audit reports.

---

## Summary for a builder

Build a Rust workspace with a C++ Clang exporter. Export each translation unit's AST, macro and conditional-compilation records, and target data layout into a versioned schema. Lower that into TIR. Run analyses that classify every pointer (owning, borrowed shared, borrowed mutable, nullable, array-with-length, unknown) and detect undefined-behavior reliance. Apply deterministic translation rules that choose the cheapest memory strategy the analysis can justify, falling back from plain safe Rust to runtime-checked safe Rust to contained `unsafe` to "keep the original C++ behind an FFI stub." Emit a Cargo crate and a report. Verify with compilation, lints, differential testing against the original, fuzzing, sanitizers and Miri. Reject anything that fails a configured safety-gate policy. Never use AI in the core path, never silently emit `unsafe`, and never claim more than was checked.
