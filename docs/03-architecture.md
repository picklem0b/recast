# 03. Architecture

## 1. System overview

recast is a pipeline of five stages around a central data structure (TIR), plus an interop subsystem and an orchestrator.

```
                         compile_commands.json + recast.toml + contracts.toml
                                          |
                                          v
 +--------------+   AST schema   +--------+   TIR    +-----------+  TIR + facts  +-------------+
 |  EXPORTER    | -------------> |FRONTEND| -------> | ANALYSIS  | ------------> | TRANSLATION |
 | (C++/Clang)  |  recast-ast/1    | (Rust) |          |  (Rust)   |               |   (Rust)    |
 +--------------+                +--------+          +-----------+               +------+------+
                                                                                        |
                                                                          Rust crate + tier map
                                                                                        v
                                                    +--------------+   verdict   +--------------+
                                                    |   INTEROP    | <---------> | VERIFICATION |
                                                    | shims, build |             |  + SAFETY    |
                                                    +--------------+             |    GATE      |
                                                                                 +------+-------+
                                                                                        v
                                                                      Output crate + verification report
```

Stages:

1. **Exporter** reads C/C++ with Clang and emits the AST schema.
2. **Frontend** lowers the schema to TIR, merges translation units, and resolves macros and configurations.
3. **Analysis** attaches facts to TIR (points-to, ownership, lifetimes, nullability, array lengths, UB findings).
4. **Translation** applies rules to choose a Rust form for every TIR item and emits source.
5. **Verification** checks the output; the **safety gate** accepts or rejects it.
6. **Interop** generates the shims and build glue that let migrated and unmigrated code coexist. It is invoked by translation (to emit shims) and by verification (to run original and translated code side by side).
7. **Orchestrator** schedules, parallelizes and caches all of the above.

## 2. Guiding rules for the whole system

These apply to every component.

### 2.1 Determinism

- All maps and sets iterated for output MUST have a defined order (use ordered collections or sort by stable key).
- No stage may read the clock, environment variables (except those declared in configuration), random sources or network.
- Parallel work MUST NOT change results; merge steps sort by stable keys.
- Solvers MUST run with fixed seeds and fixed resource limits expressed in logical steps, not wall-clock time, where the solver supports it. Where it does not, a timeout produces a recorded `unknown`, and the fallback tier is used (never a nondeterministic choice).
- The tool version, rule-set hash, input hash and toolchain versions are recorded in the report so a run can be reproduced.

### 2.2 Conservatism

- When an analysis is uncertain, it MUST choose the conservative answer: assume aliasing, assume nullable, assume ownership is unknown. Conservative results lead to higher-tier (more runtime-checked) output, never to unsound zero-cost output.
- A result is _proved_ only if the analysis can justify it from facts; a result is never promoted by heuristics alone. Heuristics MAY be used to _suggest_ a hypothesis that is then checked.

### 2.3 Locality of failure

- A failure in one function or item MUST NOT abort the run. The item degrades to Tier T4 (flagged, FFI stub) and a finding is recorded.
- Only global failures (cannot read the compilation database, cannot find Clang, schema version mismatch) abort the run.

### 2.4 Explainability

- Every decision the translator makes (tier choice, type mapping, rule application) MUST be recorded with a rule ID, the facts used and the alternatives rejected, retrievable by `recast explain`.

### 2.5 Safety of the tool itself

- recast processes untrusted source code. The exporter MUST NOT execute any code from the input. Verification stages that _do_ execute code (running original and translated programs, fuzzing) MUST run in a sandbox with no network access and limited filesystem access (see `07-verification-specification.md`).

## 3. Technology choices

| Concern                   | Choice                                                                                                                                            | Rationale                                                                                                                                                                                                                                   |
| ------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Core language             | Rust, stable toolchain                                                                                                                            | Same ecosystem as the target; memory safety in the tool; strong enum and pattern support suit IR work                                                                                                                                       |
| C/C++ front end           | Clang LibTooling in a small C++ exporter                                                                                                          | A C++ parser written from scratch is a multi-year effort. **[Research]** C2Rust parses and type-checks with Clang and consumes a Clang-derived AST; Clang LibTooling is the standard route to this and works from a `compile_commands.json` |
| Exporter output           | Versioned binary or JSON-lines schema `recast-ast/1`                                                                                              | Decouples Clang's evolving AST from the Rust core                                                                                                                                                                                           |
| Build awareness           | `compile_commands.json`                                                                                                                           | Clang tools use it to reproduce exact flags per file                                                                                                                                                                                        |
| Internal IR               | TIR (own)                                                                                                                                         | Independence from Clang's AST details; designed for analysis and for readable output                                                                                                                                                        |
| Constraint solving        | Z3 via a `ConstraintSolver` trait                                                                                                                 | **[Research]** Crown formulates ownership inference as constraints solved with a SAT/SMT solver (z3 binding)                                                                                                                                |
| Serialization             | Serde with a stable binary format (and JSON for debugging)                                                                                        | Fast cache and clear debugging                                                                                                                                                                                                              |
| Parallelism               | Work-stealing thread pool, per translation unit and per call-graph strongly connected component                                                   | Scales on many-core machines                                                                                                                                                                                                                |
| Generated code formatting | `rustfmt` with a pinned config                                                                                                                    | Stable, readable output                                                                                                                                                                                                                     |
| Verification tooling      | `cargo check`, `clippy`, `cargo test`, Miri, `cargo-fuzz` or AFL++, AddressSanitizer/UBSan/TSan on the original, optional Kani for bounded proofs | Standard ecosystem tools                                                                                                                                                                                                                    |
| Mixed-language builds     | Generated `extern "C"` shims, `cbindgen` for headers, optional `cxx` where suitable, Corrosion for CMake, `rules_rust` for Bazel                  | See `08-interop-and-build-integration.md`                                                                                                                                                                                                   |

Where the table says "optional", the builder MAY defer it; where it names a specific tool, substitute only through a recorded decision.

## 4. Repository layout

```
recast/
  Cargo.toml                    workspace manifest
  rust-toolchain.toml           pinned stable toolchain
  exporter/                     C++ project (CMake) producing recast-exporter binary
    src/
    CMakeLists.txt
  crates/
    recast-schema/                recast-ast schema types shared with the exporter (generated or hand-synced)
    recast-ir/                    TIR data structures, builders, serialization, validation
    recast-frontend/              schema -> TIR, TU merge, macro and configuration resolution
    recast-analysis/              alias, ownership, lifetime, nullability, array, UB, template analyses
    recast-translate/             rule engine, type mapping, expression/statement emitters, idiom pass
    recast-rt/                    runtime support crate shipped with generated code (see 06)
    recast-verify/                differential harness, fuzz glue, sanitizer drivers, safety gate, report
    recast-interop/               shim generator, header generation, build-system integration
    recast-orchestrator/          pipeline driver, scheduler, cache
    recast-cli/                   command-line interface
    recast-testkit/               shared test helpers, golden-file utilities
  corpus/                       real projects used for testing (git submodules or pinned archives)
  benchmarks/                   performance and accuracy tracking
  docs/                         this document set
  xtask/                        build and CI helper commands
```

Folder names from earlier design sketches map to crates as follows: `frontend/clang_bridge` -> `exporter/` and `recast-frontend`; `frontend/ir_builder` -> `recast-frontend` and `recast-ir`; `frontend/macro_resolver` -> `recast-frontend` (macro module); `analysis/*` -> `recast-analysis`; `translation/*` -> `recast-translate` and `recast-rt`; `verification/*` -> `recast-verify`; `interop/*` -> `recast-interop`; `orchestrator/*` -> `recast-orchestrator`.

### Crate dependency rules (acyclic)

```
recast-schema <- recast-frontend <- recast-analysis <- recast-translate
recast-ir     <- (recast-frontend, recast-analysis, recast-translate, recast-verify, recast-interop)
recast-rt     (standalone; depended on by generated code only, not by the tool)
recast-interop <- recast-translate, recast-verify
recast-orchestrator <- all stage crates
recast-cli <- recast-orchestrator
```

`recast-ir` MUST NOT depend on any stage crate. Stage crates MUST NOT depend on each other except as shown.

## 5. Component responsibilities

### 5.1 Exporter (`exporter/`)

- Reads one translation unit given a compile command.
- Emits: declarations (with USRs), types (with sizes and alignments from the target), function bodies as typed AST, template instantiations actually used, source ranges, attributes, and **preprocessor records** (macro definitions, macro expansion events with their argument spellings, conditional directive evaluation results, include graph).
- Records the target triple and data layout (sizes of `int`, `long`, pointers; endianness; alignment rules; whether `char` is signed).
- MUST NOT resolve or hide macros; it reports both the expanded AST and the macro trail so the frontend can reconstruct abstractions.
- Version-stamps its output with schema version and Clang version.

### 5.2 Frontend (`recast-frontend`)

- Reads exporter output for all TUs and merges items by USR (a header included in many TUs yields one item).
- Builds TIR (see `04-ir-specification.md`).
- Resolves **configurations**: determines, per source line or item, the condition under which it is active (see section 6 of `05-analysis-specification.md`); produces one TIR per configuration of interest, or one TIR with conditional items.
- Validates TIR invariants (see `04`) and fails fast on violations.

### 5.3 Analysis (`recast-analysis`)

- Computes the facts defined in `05-analysis-specification.md` and stores them in the TIR fact tables.
- Provides the _ownership decision_ for every pointer-typed place and the _UB findings_ list.
- Exposes a stable API: `analyze(tir: &mut Tir, config: &AnalysisConfig) -> AnalysisSummary`.

### 5.4 Translation (`recast-translate`)

- Chooses the tier for each function and type.
- Maps types, expressions and statements to Rust syntax trees (an internal Rust AST, not strings), then pretty-prints.
- Runs the idiom pass.
- Emits the Cargo workspace for the output and the tier map.
- Records the explanation for every choice.

### 5.5 Runtime support (`recast-rt`)

- A small Rust crate that generated code depends on. Contains the checked cell types, byte string, exception type, C-library shims and boundary helpers. See `06-translation-specification.md` section 9.
- MUST be `#![forbid(unsafe_code)]` except in clearly separated modules (`ffi`, `raw`) that wrap unavoidable `unsafe` with documented invariants.

### 5.6 Verification and safety gate (`recast-verify`)

- Implements the verification ladder and the gate. See `07-verification-specification.md`.

### 5.7 Interop (`recast-interop`)

- Generates `extern "C"` shims, C headers (via cbindgen-style generation), layout assertions and build glue. See `08-interop-and-build-integration.md`.

### 5.8 Orchestrator (`recast-orchestrator`)

- Drives the stages, parallelizes per translation unit and per call-graph SCC, caches by content hash, and records timings.

## 6. Data flow and artifacts

All intermediate artifacts are written under the output directory (`recast-out/` by default; see `09-cli-and-configuration.md`) so any stage can be re-run or inspected.

| Stage        | Reads                              | Writes                                                  |
| ------------ | ---------------------------------- | ------------------------------------------------------- |
| exporter     | source, compile command            | `ast/<tu-hash>.tast`                                    |
| frontend     | `ast/*.tast`, config               | `tir/program.tir` (+ `tir/program.tir.json` on request) |
| analysis     | `tir/program.tir`, contracts       | `tir/program.facts`                                     |
| translation  | `tir/program.tir`, `program.facts` | `rust/` crate, `tiermap.json`, `explain/*.json`         |
| interop      | tier map, TIR                      | `shims/`, `include/`, build glue                        |
| verification | original + `rust/` + shims         | `verify/*.json`, `report.json`, `report.md`             |

Cache keys use the SHA-256 of (tool version, schema version, rule-set hash, config subset relevant to the stage, input content hashes).

## 7. Concurrency model

- **Exporter runs** are independent per TU; run in parallel up to `jobs`.
- **Frontend merge** is a deterministic reduction: sort TUs by normalized path, merge in that order.
- **Analysis**: intraprocedural parts run in parallel per function; interprocedural parts operate on the call-graph condensed into SCCs, processed bottom-up in parallel across independent SCCs, with a deterministic fixed-point iteration inside each SCC.
- **Translation** runs per function and per type in parallel, with a deterministic assembly step.
- **Verification** runs test shards in parallel in sandboxes; results are merged by stable test ID.

## 8. Error model

Errors are values, never panics, in the pipeline. Each stage returns `Result<Output, StageError>` plus a stream of `Finding`s.

```
Finding {
  id: FindingId,            // stable: stage + rule + location hash
  severity: Info | Note | Warning | Error,
  kind: FindingKind,        // e.g. UbReliance, UnsupportedConstruct, SolverUnknown, ContractViolation
  rule: RuleId,
  location: SourceLocation, // original file, line, column
  message: String,          // short, human-readable
  details: Json,            // structured payload
  disposition: Translated(Tier) | Flagged | Rejected | None
}
```

Panics inside recast indicate a recast bug. The orchestrator MUST catch them at the item boundary, convert them to an internal-error finding with the item degraded to T4, and mark the run as `degraded` in the report. A run with any internal error MUST NOT report a clean verdict.

## 9. Extensibility points

- **New rules**: translation rules are registered in a rule table keyed by pattern; each has an ID, a precondition on TIR plus facts, and a rewrite. Adding a rule requires a worked example in `14-worked-examples.md` and golden tests.
- **New analyses**: implement the `Analysis` trait (inputs: TIR + prior facts; output: facts) and declare dependencies; the orchestrator orders them.
- **Solver backends**: implement `ConstraintSolver`.
- **Verification rungs**: implement `VerificationRung` with a declared strength and cost class.
- **Optional AI proposer** (late milestone): implements `Proposer`, can only emit _hypotheses_ (for example an ownership guess) that are rechecked by the normal analysis and verification path. See ADR-003.

## 10. Versioning and compatibility

- Schema versions (`recast-ast/N`, `tir/N`, `report/N`) are integers; breaking changes bump N. The reader MUST refuse unknown major versions with a clear message.
- The rule set has a content hash recorded in every report.
- Output Rust targets a declared Rust edition and minimum supported Rust version (MSRV), recorded in `recast.toml` and the generated `Cargo.toml`.

## 11. Security and sandboxing

- The exporter runs with read-only access to the source tree and no network.
- Verification that runs program code uses a sandbox (container or equivalent) with: no network, read-only source mounts, a scratch directory, CPU and memory limits, and a wall-clock cap per test.
- recast never uploads source code anywhere. All processing is local to the machine or the customer's own infrastructure. An optional AI layer, if enabled in a future milestone, MUST be opt-in per project and MUST support self-hosted models.
