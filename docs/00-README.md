# recast Foundation Documents

**recast** is a C/C++ to memory-safe Rust translator with a built-in verification system. It reads an existing C or C++ codebase, builds a semantic model of how the code uses memory, writes Rust that preserves the original behavior, and then checks that claim with tests, fuzzing, sanitizers and (where feasible) formal tools. Its output is a Rust crate plus a machine-readable verification report.

These documents are the complete specification for building recast. They are written to be self-contained: a reader (human or AI model) with no other context should be able to build the system from them.

---

## How to read this set

| #   | File                                   | Purpose                                                                                        | Read when                                     |
| --- | -------------------------------------- | ---------------------------------------------------------------------------------------------- | --------------------------------------------- |
| 00  | `00-README.md`                         | This index, conventions, status                                                                | First                                         |
| 01  | `01-vision-and-scope.md`               | Problem, product definition, goals, non-goals, success metrics, supported language subset      | Before anything else                          |
| 02  | `02-glossary.md`                       | Precise definitions of every term used                                                         | Whenever a term is unclear                    |
| 03  | `03-architecture.md`                   | Components, data flow, repo layout, technology choices, cross-cutting rules                    | Before writing any code                       |
| 04  | `04-ir-specification.md`               | The internal representation (TIR) that every stage reads and writes                            | Before frontend, analysis or translation work |
| 05  | `05-analysis-specification.md`         | Alias, ownership, lifetime, nullability, array, UB, template and macro analyses                | Before analysis work                          |
| 06  | `06-translation-specification.md`      | Exact C/C++ to Rust mapping rules, memory strategy tiers, runtime support crate, output layout | Before translation work                       |
| 07  | `07-verification-specification.md`     | Verification ladder, safety gate, report schema, determinism rules                             | Before verification work                      |
| 08  | `08-interop-and-build-integration.md`  | Mixed C++/Rust builds, FFI shims, incremental migration protocol                               | Before interop work                           |
| 09  | `09-cli-and-configuration.md`          | Commands, `recast.toml`, contracts file, exit codes, output directory layout                   | Before CLI work                               |
| 10  | `10-testing-and-corpus.md`             | Test pyramid, golden tests, corpus selection, benchmarks, CI                                   | Before writing tests                          |
| 11  | `11-roadmap-and-milestones.md`         | Ordered milestones with acceptance criteria                                                    | When planning the build order                 |
| 12  | `12-risks-decisions-open-questions.md` | Risk register, decision log (ADRs), open questions                                             | When making a design choice                   |
| 13  | `13-research-and-references.md`        | Prior art, research findings, market and regulatory context, sources                           | When justifying a decision                    |
| 14  | `14-worked-examples.md`                | Concrete C/C++ inputs with expected Rust outputs and tiers                                     | When implementing any rule                    |
| 15  | `15-builder-brief.md`                  | Operating instructions for the AI model or engineer doing the build                            | Before starting; re-read when stuck           |

Recommended order for a builder: 15, 01, 02, 03, 04, then the specification for the stage being built (05, 06, 07, 08), with 14 open alongside.

---

## Conventions

### Requirement language

The words **MUST**, **MUST NOT**, **SHOULD**, **SHOULD NOT** and **MAY** are used as in RFC 2119.

- **MUST / MUST NOT**: required for correctness or for the product's trust claims. Violating it is a bug.
- **SHOULD / SHOULD NOT**: the default; deviate only with a recorded reason (see `12-risks-decisions-open-questions.md`).
- **MAY**: optional.

### Evidence labels

Every non-trivial claim in these documents carries one of three provenance labels, so a reader knows how much weight to put on it:

- **[Research]**: stated by a published paper, project page or official source. Listed in `13-research-and-references.md`.
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

### Language and tooling decisions in one place

| Concern                      | Choice                                                                             |
| ---------------------------- | ---------------------------------------------------------------------------------- |
| Core implementation language | Rust (stable toolchain, pinned by `rust-toolchain.toml`)                           |
| C/C++ parsing                | Clang, through a small C++ exporter (LibTooling) that emits a versioned AST schema |
| Primary input scope          | See "Supported language subset" in `01-vision-and-scope.md`                        |
| Generated code target        | Rust 2021 edition or later, stable toolchain, no nightly features                  |
| Constraint solving           | An SMT/SAT solver behind a trait (Z3 by default)                                   |
| AI in the core               | None. See ADR-003 in `12-risks-decisions-open-questions.md`                        |

---

## Status of this document set

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

Version: 1.0 (foundation). Date: 24 October 2026.

---

## One-paragraph summary for a builder in a hurry

Build a Rust workspace with a C++ Clang exporter. Export each translation unit's AST, macro and conditional-compilation records, and target data layout into a versioned schema. Lower that into TIR. Run analyses that classify every pointer (owning, borrowed shared, borrowed mutable, nullable, array-with-length, unknown) and detect undefined-behavior reliance. Apply deterministic translation rules that choose the cheapest memory strategy the analysis can justify, falling back from plain safe Rust to runtime-checked safe Rust to contained `unsafe` to "keep the original C++ behind an FFI stub." Emit a Cargo crate and a report. Verify with compilation, lints, differential testing against the original, fuzzing, sanitizers and Miri. Reject anything that fails a configured safety-gate policy. Never use AI in the core path, never silently emit `unsafe`, and never claim more than was checked.
