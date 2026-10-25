# 07. Verification Specification

**Trust is the product.** A translator that is mostly right is not usable in safety-critical software. This document defines how recast establishes and reports what it has and has not shown about each translated item, and how the safety gate turns those results into an accept/reject decision.

Status: rungs V0 to V4 are **Specified**. V5 is **Research-grade**. The report schema is **Specified**.

---

## 1. Principles

1. **Claims match evidence.** The report may only state a claim that a specific verification rung established for that item. No aggregate claim exceeds the weakest evidence behind it.
2. **Tests show the presence of differences, not their absence.** Passing differential tests and fuzzing is evidence of agreement on the inputs tried; it is never described as proof.
3. **The original is the oracle.** Behavior of the original program (compiled with the project's own build, plus sanitizers) defines expected behavior, except where the original exhibits UB; in that case the UB policy governs (document 05 section 10.3).
4. **Verification is separate from translation.** The verifier MUST NOT trust analysis facts. It may use them to guide test generation, never to skip checks.
5. **Deterministic and reproducible.** Seeds, tool versions, test inputs and environment are recorded so any result can be reproduced.
6. **Safe execution.** All code execution runs in a sandbox (section 9).

---

## 2. Observable behavior: what "same" means

Two programs agree on an input when their **observable behavior** is equal:

| Program kind | Observable behavior                                                                                                                                                                                            |
| ------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Executable   | Exit status; bytes written to stdout and stderr; files created or modified (content and name); network I/O in a recorded test environment; signals/abnormal termination class (clean exit vs crash vs timeout) |
| Library      | For each public function: return value, values written through output parameters or visible buffers, errors returned, state changes observable through the public API, and abnormal termination class          |
| Both         | Resource-level effects visible to tests, such as memory freed (leak counts under a leak checker are reported but are not a divergence unless the original had none)                                            |

**Exclusions** (documented, configurable): timing; memory addresses and pointer values (except through round trips); unspecified iteration orders (for example `unordered_map`); thread scheduling order; uninitialized padding bytes; and any behavior the UB catalogue classifies as undefined in the original.

**Comparison functions** are pluggable per test: byte equality (default), normalized text equality (strip timestamps, pointers), structural equality for serialized state, and user-provided comparators.

---

## 3. The verification ladder

Each rung is cheaper and weaker, or costlier and stronger. A rung's result is recorded per item (function, type, global) wherever the rung can attribute results to items, and per crate otherwise.

| Rung   | Name                          | What it checks                                                                                                                                                                                                                          | Evidence strength                         | Cost           |
| ------ | ----------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------- | -------------- |
| **V0** | Build and syntax              | Generated Rust compiles (`cargo check`), formats (`rustfmt --check`), and the T0 to T2 modules carry `#![forbid(unsafe_code)]`                                                                                                          | Weak (necessary)                          | Low            |
| **V1** | Static hygiene                | `clippy` with a configured deny-list; unsafe inventory (count and location of every `unsafe` block, via a syntax-tree scan); layout assertions compile and hold for FFI types; no forbidden APIs (`static mut`, `transmute` outside T3) | Weak to medium                            | Low            |
| **V2** | Differential testing          | Original tests and generated characterization tests produce identical observable behavior on original vs translated code                                                                                                                | Medium                                    | Medium         |
| **V3** | Differential fuzzing          | Coverage-guided fuzzing generates inputs; both implementations run; outputs compared                                                                                                                                                    | Medium to strong (for the explored space) | High, budgeted |
| **V4** | Dynamic safety checks         | Sanitizers on the **original** reveal UB reliance; Miri and overflow-checked runs on the **translated** code check `unsafe` and arithmetic                                                                                              | Medium; targeted                          | Medium         |
| **V5** | Bounded or formal equivalence | For selected pure or small functions: bounded model checking or translation-validation style refinement checks                                                                                                                          | Strong (within bounds)                    | Very high      |

A run executes every rung enabled by the configured **profile** (section 6). Skipped rungs are listed as skipped, never silently omitted.

### 3.1 V0: build and syntax

- `cargo check --all-targets` with the pinned toolchain and every feature combination in the configuration matrix.
- `rustfmt --check` (generation uses rustfmt; check guards against drift).
- Verify that every module file for T0 to T2 begins with `#![forbid(unsafe_code)]` and that the compiler accepts it (it enforces it).
- Failure on any item means that item is not accepted; the item degrades to T4 if a fallback exists, otherwise the run is rejected.

### 3.2 V1: static hygiene

- `cargo clippy` with deny on `clippy::correctness` and a configured subset of `suspicious`/`perf`; allowed warnings recorded.
- **Unsafe inventory**: parse the generated code (via `syn`), list every `unsafe` block/function/impl with location, enclosing function, and the `SAFETY` comment (required for T3). A T3 block without a `SAFETY` comment fails V1.
- **Layout assertions**: for each `#[repr(C)]` type exposed across FFI, generated assertions (`const _: () = assert!(size_of::<T>() == N)`, offsets via `core::mem::offset_of!`) must hold and must match the C++ side values (document 08 section 5).
- **Forbidden patterns** (configurable): `static mut`, `transmute` outside T3, `from_raw_parts` outside `raw`/`ffi`, `unwrap()` in generated library code outside tests (generated code uses explicit panics with source-located messages instead).

### 3.3 V2: differential testing

Three test sources, in order of preference.

**(a) Link-swap testing (libraries).** Rebuild the _original project's own test suite_ against the translated library through the generated C ABI shims, in place of the original implementation, and run it unchanged. This reuses the customer's existing tests and answers the real question: does the project still pass its own tests? Original-vs-translated pass/fail sets must be identical. Requires interop (document 08).

**(b) Whole-program tests (executables).** Run both binaries on the same command lines, stdin, environment and working-directory contents; compare observable behavior (section 2). Test inputs come from existing test scripts, recorded example sessions, and generated variations.

**(c) Characterization tests (functions).** For projects with weak test coverage:

1. Build an instrumented version of the original in which selected functions log arguments and results (deep-copied through a serializer generated from TIR types; pointers serialized as pointee contents using extents; opaque pointers as handles).
2. Run the original's available tests, examples and fuzz corpora to collect call records.
3. Generate Rust tests that replay each recorded call against the translated function and compare results and side-effect observations.
4. De-duplicate records by input equivalence; cap per-function records by budget; keep a manifest with hashes so runs are reproducible.

**Nondeterminism handling**: each test is run twice against the original; if outcomes differ, the test is classified `nondeterministic-original`, excluded from pass/fail, and listed. Time, randomness and addresses are controlled by fixed seeds and environment where possible (`TZ`, `LANG`, `SOURCE_DATE_EPOCH`, seeded RNGs).

**Coverage**: collect line/branch coverage of both implementations (`llvm-cov`/`gcov` on the original, `cargo llvm-cov` on Rust). The report includes the share of translated functions executed by V2 tests. Functions with no test coverage are listed as **unexercised** and the gate treats them according to profile.

### 3.4 V3: differential fuzzing

- **Harness generation**: for each function that is externally callable or recognized as an input-processing entry point (parsers, decoders, deserializers, command handlers), generate a fuzz target that takes bytes, derives arguments through a structure-aware decoder built from TIR types, calls **both** implementations (original via shim, translated natively), and compares results with the comparison functions of section 2. Divergence aborts with the input saved.
- **Engines**: libFuzzer or AFL++ for the C++ side builds; `cargo-fuzz` for Rust; the combined harness is built as a single process linking both (this is why C ABI shims exist).
- **Budget and determinism**: a fixed iteration count or CPU-time budget per target, a recorded seed corpus (from tests and examples), and a recorded RNG seed. The report states the budget and coverage achieved.
- **Triage**: every divergence is stored as a minimized reproducer plus the diff; classified as `translation-bug`, `original-UB` (confirmed by sanitizer), `nondeterminism`, or `unknown`. `translation-bug` and `unknown` fail the gate.
- **Corpus reuse**: seed corpora from the original project's fuzzing (if any) are imported.

### 3.5 V4: dynamic safety checks

- **Original under sanitizers**: build the original with ASan, UBSan (with the full recommended set, noting checks that are not UB-related), MSan where the toolchain supports it, and run V2 tests and V3 corpora. Each sanitizer report is mapped to a UB finding (`UB-nnn`) by location. If the original has a _proven_ UB at a site, the translation's defined behavior is not considered a divergence, but the site is listed in the report with the original's observed behavior.
- **Original under ThreadSanitizer** when the code is multi-threaded.
- **Translated code**:
  - run tests with `debug-assertions` and `overflow-checks` enabled (to expose arithmetic that panics where the original wrapped or invoked UB),
  - run T3 and `ffi` code under **Miri** (where Miri supports the operations; unsupported operations noted),
  - run tests under AddressSanitizer for the Rust crate when it contains FFI calls into C++ (to catch boundary errors).
- Panics in translated code on tests where the original produced output are divergences and fail the gate unless the policy classifies them as expected UB sites with a confirmed sanitizer finding in the original.

### 3.6 V5: bounded and formal evidence (Research-grade)

Optional, for selected functions (pure, loop-bounded, small):

- **Bounded model checking of the Rust function** (for example with Kani) for absence of panics, overflow and out-of-bounds within configured bounds.
- **Equivalence by refinement checking**: lower both implementations to a common low-level form (for example LLVM IR for both the original C++ function and the translated Rust function) and check that every behavior of the translated function is a behavior of the original on defined inputs, using an SMT-based refinement checker in the style of LLVM translation-validation tools. Practical only for small side-effect-free functions without unbounded loops or complex memory.
- Claims from V5 name the bounds ("equivalent for all inputs with array length <= 8 and elements in i16 range").

V5 results upgrade the _evidence class_ of an item (section 5) but are never required by the default profiles.

---

## 4. Safety gate

The safety gate consumes all evidence and the translation's own record (tiers, unsafe inventory, flagged items, UB sites) and returns a verdict per item and per run.

### 4.1 Verdict values

- **ACCEPT**: every required check passed; no items beyond profile limits.
- **ACCEPT_WITH_NOTES**: passed, but contains items the profile allows with disclosure (for example T2 runtime checks, T3 blocks, assumed facts, unexercised functions).
- **REJECT**: a required check failed or a profile limit was exceeded.
- **DEGRADED** (run-level only): an internal recast error occurred in some item (document 03 section 8); a run is never ACCEPT while DEGRADED.

### 4.2 Item-level decision procedure

```
for each item:
  if V0 or V1 failed                                  -> degrade to T4 (if the item is flaggable) else REJECT item
  if V2 divergence                                    -> REJECT item (degrade to T4 if profile allows fallback)
  if V3 divergence of class translation-bug|unknown   -> REJECT item (as above)
  if V4 finds panic where original produced output
        and no confirmed UB in original               -> REJECT item
  evidence_class = strongest passing rung for the item
  if evidence_class below the profile minimum for the item's tier -> REJECT or NOTE per profile
  else -> ACCEPT or ACCEPT_WITH_NOTES (notes: assumptions, T2/T3, UB sites, unexercised)
```

"Degrade to T4" means regenerate the item as an FFI stub of the original and re-verify; this is the only automatic repair, and it always preserves behavior because it _is_ the original code.

### 4.3 Run-level decision

- REJECT if any item is REJECT and has no valid T4 fallback.
- Otherwise ACCEPT_WITH_NOTES if any item has notes; ACCEPT only if all items are plain T0 to T1 with passing evidence at or above profile minimums and no notes (rare for real codebases; expected for small modules).
- The run's verdict never hides item-level verdicts; both appear in the report.

### 4.4 Profiles

Profiles are named policy bundles (`recast.toml`: `[gate] profile = "standard"`). Users can define their own.

| Rule                                | `strict`                              | `standard` (default)                       | `permissive` | `audit`                             |
| ----------------------------------- | ------------------------------------- | ------------------------------------------ | ------------ | ----------------------------------- |
| Max tier allowed                    | T1 (T2 with disclosure if `allow_t2`) | T2                                         | T3           | T4 (report-only)                    |
| T3 blocks                           | Rejected                              | Allowed with SAFETY comments and Miri pass | Allowed      | Allowed                             |
| T4 flagged items                    | Rejected                              | Allowed (listed; build continues)          | Allowed      | Allowed                             |
| Required rungs                      | V0, V1, V2, V3, V4                    | V0, V1, V2, V4                             | V0, V1       | None enforced; all run and reported |
| Unexercised functions               | Rejected                              | Allowed with note                          | Allowed      | Reported                            |
| Assumed facts (`Likely`, contracts) | Must be verified by a rung            | Allowed with note                          | Allowed      | Reported                            |
| UB policy                           | `strict` (proven UB sites -> T4)      | `report`                                   | `report`     | `report`                            |
| Divergence on any rung              | Reject item                           | Reject item                                | Reject item  | Reported, not enforced              |
| Required determinism check          | Yes                                   | Yes                                        | Yes          | Yes                                 |

The `audit` profile is intended for evaluation: it never rejects, it just measures, so a customer can see what a stricter profile would have done.

### 4.5 Gate rules (identifiers)

| ID       | Rule                                                                                               |
| -------- | -------------------------------------------------------------------------------------------------- |
| GATE-001 | A crate declared safe (`#![forbid(unsafe_code)]` everywhere) must contain no `unsafe` tokens       |
| GATE-002 | Every `unsafe` block outside `ffi`/`raw` has a SAFETY comment and a test                           |
| GATE-003 | Every T4 stub has a boundary test (call through the Rust API equals call through the original)     |
| GATE-004 | No `REJECT` item is present in the delivered crate without a T4 fallback                           |
| GATE-005 | Determinism check passed (two builds identical)                                                    |
| GATE-006 | Every assumed fact used for T0 is verified by at least one rung (`strict`) or listed (`standard`)  |
| GATE-007 | Every UB site with proven or likely confidence is listed with the original and translated behavior |
| GATE-008 | Layout assertions pass for every exported type                                                     |
| GATE-009 | The verification environment is recorded (toolchains, sanitizer versions, sandbox config)          |

---

## 5. Evidence classes and claim language

For every item the report assigns exactly one **evidence class**, and uses only the approved wording.

| Class | Meaning                                                        | Approved wording                                                                                             |
| ----- | -------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------ |
| `E0`  | Compiles; not otherwise checked                                | "Translated; compiles."                                                                                      |
| `E1`  | Hygiene checks pass                                            | "Translated; static checks pass."                                                                            |
| `E2`  | Differential tests pass; function was exercised                | "Matches the original on N test executions covering X% of the function."                                     |
| `E3`  | E2 plus differential fuzzing within budget with no divergences | "No divergence found in N fuzz executions (budget B, seed S)."                                               |
| `E4`  | E3 plus sanitizer/Miri checks clean                            | "Dynamic checks found no memory-safety violation in the original or translated code on the executed inputs." |
| `E5`  | Bounded or formal evidence                                     | "Equivalent to the original for all inputs within bounds <bounds>."                                          |

Forbidden wording: "proven correct," "guaranteed identical," "safe" (without qualification), or any claim stronger than the item's evidence class. The word "safe" is used only for "contains no `unsafe`," which is compiler-enforced.

---

## 6. Report

### 6.1 Outputs

- `report.json` (canonical, machine-readable, stable key order)
- `report.md` (human-readable rendering)
- `report.sarif` (optional: findings in SARIF for IDE and CI integration)
- `attestation.json` (optional signed statement; see 6.5)

### 6.2 JSON structure (schema `recast-report/1`)

```json
{
  "schema": "recast-report/1",
  "run": {
    "tool_version": "1.0.0",
    "rule_set_hash": "sha256:...",
    "schema_versions": { "ast": 1, "tir": 1 },
    "input": {
      "source_hash": "sha256:...",
      "compile_commands_hash": "sha256:...",
      "config_hash": "sha256:...",
      "contracts_hash": "sha256:..."
    },
    "toolchains": { "clang": "...", "rustc": "...", "cargo": "...", "z3": "..." },
    "target": { "triple": "x86_64-unknown-linux-gnu", "int_bits": 32, "pointer_bits": 64 },
    "profile": "standard",
    "started_at": "2026-10-24T17:00:00Z",
    "environment": { "os": "...", "cpu": "...", "jobs": 16 }
  },
  "verdict": "ACCEPT_WITH_NOTES",
  "determinism": { "checked": true, "output_hash": "sha256:...", "runs": 2, "identical": true },
  "summary": {
    "items": { "total": 1240, "translated": 1198, "flagged_t4": 42, "rejected": 0 },
    "by_tier": { "T0": {"functions": 880, "loc": 41200}, "T1": {...}, "T2": {...}, "T3": {...}, "T4": {...} },
    "unsafe": { "generated_code_blocks": 11, "runtime_crate_blocks": 7, "ffi_boundary_blocks": 42, "unsafe_per_kloc": 0.31 },
    "runtime_overhead": { "method": "...", "median_ratio": 1.04, "worst_function_ratio": 6.1, "benchmarks": [...] },
    "coverage": { "functions_exercised_pct": 87.4, "lines_pct": 79.0 },
    "macro_preservation": { "uses_total": 4210, "preserved_as_abstraction": 3722, "expanded": 488 },
    "configurations": [ { "id": "linux-release", "status": "translated" } ]
  },
  "evidence": { "E0": 3, "E1": 11, "E2": 790, "E3": 360, "E4": 34, "E5": 0 },
  "items": [
    {
      "id": "fn:c:@F@sum#*1I#l#",
      "source": { "file": "src/util.c", "line": 41 },
      "rust": { "path": "util::sum", "file": "src/util.rs", "line": 22 },
      "tier": "T0",
      "rules": ["TR-201", "TR-310"],
      "verdict": "ACCEPT",
      "evidence_class": "E3",
      "evidence": [
        { "rung": "V0", "result": "pass" },
        { "rung": "V2", "result": "pass", "executions": 312, "coverage_pct": 100 },
        { "rung": "V3", "result": "pass", "executions": 5000000, "seed": 91823, "budget": "300s" }
      ],
      "assumptions": [ { "fact": "NUL-002", "text": "a is non-null (3/3 call sites)", "verified_by": "V2" } ],
      "ub_sites": [],
      "notes": []
    }
  ],
  "flagged": [ { "id": "...", "reason": "UNSUPPORTED:setjmp", "stub": "ffi::wrap_decode", "evidence": [ ... ] } ],
  "ub_sites": [ { "id": "UB-001", "location": {...}, "confidence": "Likely", "original_behavior": "wraps (observed)", "translated_behavior": "panics (overflow-checks)", "policy": "report" } ],
  "unsafe_inventory": [ { "file": "...", "line": 10, "fn": "...", "tier": "T3", "safety_comment": "...", "tests": ["..."], "miri": "pass" } ],
  "divergences": [ { "id": "...", "class": "original-UB", "reproducer": "fuzz/crash-0f3a", "resolution": "..." } ],
  "skipped_rungs": [ { "rung": "V5", "reason": "disabled by profile" } ],
  "findings": [ ... ]
}
```

- Keys MUST be emitted in a fixed order; arrays sorted by stable key (item `id`).
- `started_at` and `environment` live under `run` and are excluded from the determinism hash (see 8).

### 6.3 Metric definitions

- **Function coverage by tier**: functions (translated) whose max tier is `Tn`, divided by total functions in scope.
- **LOC coverage by tier**: physical source lines of those functions, from the original source, divided by total in-scope lines (blank and comment lines excluded).
- **Unsafe density**: `unsafe` blocks (or `unsafe` function/impl declarations) in generated code per thousand lines of generated Rust code, reported separately for generated code, `recast-rt`, and FFI boundary.
- **Runtime overhead**: measured on the project's benchmarks (or a user-provided benchmark script): median of N runs (N >= 10), pinned to a single CPU where possible, release builds on both sides with the same optimization level, environment recorded. Overhead is **excluded from the determinism hash**.
- **Macro preservation**: share of macro use sites translated as abstractions instead of expanded (document 05 section 7).

### 6.4 Human-readable report (`report.md`) structure

1. Verdict and one-paragraph summary (what was established, what was not).
2. Key numbers table (tiers, unsafe, overhead, coverage, evidence classes).
3. What needs human attention: flagged items, T3 blocks, UB sites, divergences, unexercised functions, assumptions.
4. Per-module table of items with tier, evidence class, notes.
5. Reproducibility block: versions, hashes, commands to reproduce.
6. Limitations statement (verbatim, always present): states that tests and fuzzing do not prove equivalence, lists excluded observables, and lists rungs not run.

### 6.5 Attestation (optional)

`attestation.json` contains: report hash, output crate hash, tool version, and a signature (Ed25519) from a configured key. It lets a customer prove to an auditor that a given crate came from a given run. Key management is the customer's responsibility; recast never generates keys implicitly.

---

## 7. Test and verification artifacts

Under `recast-out/verify/`:

```
verify/
  v0/ build logs, fmt results
  v1/ clippy json, unsafe_inventory.json, layout_check.json
  v2/ linkswap/ whole_program/ characterization/ coverage/
  v3/ targets/ corpora/ crashes/ minimized/
  v4/ sanitizers_original/ miri/ overflow_runs/
  v5/ kani/ refinement/        (when enabled)
  gate.json
```

Every file records the command, tool versions and seed that produced it.

---

## 8. Determinism requirements for verification

- The **translation output** and the **report except metadata** MUST be byte-identical across runs given the same inputs. The determinism check (GATE-005) performs two full translations (or one translation and a cached-rebuild comparison) and compares hashes of: the generated crate tree, the tier map, the explain records, and the canonical report with `run.started_at`, `run.environment`, `summary.runtime_overhead` and wall-clock fields removed.
- **Fuzzing and tests** are deterministic given seeds; the report records seeds and budgets. Different machines may explore different paths under time budgets; therefore budgets are expressed in **execution counts** by default, not seconds.
- Flaky results (a test passing and failing across reruns) are retried exactly twice, recorded as `flaky`, and excluded from pass/fail only if flaky in the original too; otherwise they count as failures.

---

## 9. Sandbox for executing code

All stages that run program code (tests, fuzzing, sanitizer runs, benchmarks) MUST run in a sandbox:

- No network access (a loopback-only network namespace for tests that need local sockets).
- Read-only mount of the source tree; a per-test writable scratch directory, deleted afterwards.
- Resource limits: CPU time, wall-clock, memory, file size, process count.
- No access to the host's home directory, credentials or environment variables beyond an explicit allowlist.
- Seccomp or equivalent syscall filtering recommended; containers (rootless) are the reference implementation.
- A kill-on-timeout policy: tests exceeding time limits are recorded as `timeout`; a timeout in the original but not the translation is **not** counted as a translation failure and is reported for human review.

---

## 10. Benchmarking protocol (overhead measurement)

1. Choose benchmarks: the project's own, else a user-provided script, else generated micro-benchmarks around hot functions identified by profiling the original on its tests.
2. Build original and translated in release mode with comparable optimization (`-O2` or `-O3` vs `opt-level=3`), LTO off or on both, frame pointers consistent.
3. Run each benchmark N times interleaved (A, B, A, B, ...) to reduce drift; discard the first run; report median and interquartile range.
4. Record the CPU model, governor, OS, and load.
5. Report overhead per function (when profiling allows) and per benchmark; flag any function with overhead above a configurable threshold (default 2x) as a **performance regression candidate** with its tier explanation (for example "T2 RefCell due to aliasing between `p` and `q` at util.c:88").

---

## 11. What verification does not do

- It does not prove the original code is correct.
- It does not establish equivalence on inputs it never ran (except V5 within bounds).
- It does not detect behavior that depends on undefined behavior in a way no sanitizer reports.
- It does not validate performance requirements beyond measuring overhead.
- It does not certify compliance with a specific standard (for example ISO 26262 or DO-178C); the report is _evidence_ that can feed such a process.

These limits are printed in every report.
