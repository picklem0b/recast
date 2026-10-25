# 10. Testing and Corpus

How recast itself is tested. A translator is only as trustworthy as its own test suite. This document defines test layers, the benchmark corpus, the metrics tracked, and the release criteria.

Status: **Specified**. Corpus contents are named targets; the builder selects exact versions and pins them.

---

## 1. Test layers

| Layer                      | Scope                                                              | Examples                                                                  | Runs                                   |
| -------------------------- | ------------------------------------------------------------------ | ------------------------------------------------------------------------- | -------------------------------------- |
| L1 Unit                    | One function or module in one crate                                | IR validators, solver encodings, format-string parser, rule preconditions | Every commit                           |
| L2 Golden                  | Input C/C++ snippet to expected Rust output, byte-exact            | One golden per translation rule (`TR-nnn`), per idiom (`IDM-nnn`)         | Every commit                           |
| L3 Analysis ground truth   | Snippet plus hand-labeled expected facts                           | Ownership, nullability, extent, alias, UB labels                          | Every commit                           |
| L4 Behavioral differential | Snippet compiled and run in both languages, outputs compared       | Arithmetic, control flow, strings, containers                             | Every commit (small), nightly (large)  |
| L5 Property-based          | Generated programs from a grammar; compare original and translated | Random expression trees, random struct/array manipulations                | Nightly                                |
| L6 Corpus                  | Real open-source projects translated end to end                    | See section 3                                                             | Nightly / weekly                       |
| L7 Soundness validation    | Dynamic check that `Proven` facts never contradicted               | Shadow-tracking instrumented runs                                         | Weekly                                 |
| L8 Robustness              | Malformed, huge, hostile inputs                                    | Crash and resource tests                                                  | Nightly                                |
| L9 Determinism             | Same inputs, two runs, various `--jobs`                            | Byte-compare outputs                                                      | Every commit (small), nightly (corpus) |
| L10 Performance of recast  | Throughput and memory of the tool                                  | Lines per second per stage                                                | Weekly                                 |

### 1.1 L2 golden tests

Layout: `tests/golden/<rule-id>/<case>/{input.c|cpp, config.toml, expected.rs, expected.explain.json}`. Updating a golden requires a reviewed diff; a tool (`xtask bless`) regenerates files, and CI forbids bless in the same change that edits the rule unless the diff is committed.

Every rule in documents 05 and 06 MUST have at least: one positive case, one negative case (precondition not met, so a different tier or flag), and one boundary case.

### 1.2 L3 analysis ground truth

Annotated sources use comments:

```c
void f(int *p /* @own=Borrowed @null=NonNull @extent=Len(n) */, size_t n) { ... }
```

The harness parses annotations, runs analysis, and compares. Mismatch categories:

- **Unsound** (claimed `Proven` but wrong): P0.
- **Imprecise** (`Unknown` where answer is determinable): tracked metric.
- **Crash or timeout**: P1.

### 1.3 L4 behavioral differential

For each snippet: compile with the reference compiler (Clang, `-O0` and `-O2`), translate, build Rust (debug and release), execute with a fixed input set, compare observable behavior (document 07 section 2). Snippets with UB are excluded from L4 and covered by L3/L7 and the UB-policy tests.

### 1.4 L5 property-based

Grammar-driven generators for a defined safe subset (no UB by construction: bounded arithmetic, in-range indexes, initialized variables). Compare outputs. Shrink failures to minimal reproducers and add them to L2/L4 permanently.

### 1.5 L7 soundness validation

Instrument original programs with a shadow runtime that tracks, per allocation, the set of live pointers and who freed it, and per pointer, null-ness and bounds. Run the corpus test suites. Any dynamic observation contradicting a `Proven` fact fails the build and is filed as P0.

### 1.6 L8 robustness

- Fuzz the exporter schema reader and the frontend with mutated `recast-ast/1` files.
- Pathological inputs: deeply nested expressions, huge arrays, 100k-line generated files, giant macros, recursion in templates, circular includes, invalid UTF-8 in comments and strings, `#include` of missing files.
- Resource limits: tool MUST fail with a clear error, not hang or exhaust memory (limits configurable).

### 1.7 L9 determinism

Run each corpus project twice with different `--jobs`, different working directory, different environment ordering, and compare hashes (document 07 section 8). Any difference is a P0.

---

## 2. Test infrastructure (`recast-testkit`)

- Snippet harness: compile with Clang, run, capture observable behavior.
- Golden runner and bless tool.
- Annotation parser for L3.
- Shadow-runtime library (L7) for C and C++.
- Property-test generators and shrinkers.
- Corpus runner with per-project config, expected-verdict manifests, and result history database (SQLite) for trend tracking.
- Differential comparer (byte, normalized, structural).
- Flakiness tracker: tests that change outcome without code change are quarantined and counted.

---

## 3. Corpus

Selection criteria: open licenses permitting use in tests, buildable with CMake or Make, own tests present, range of size and style, widely known to avoid cherry-picking.

### 3.1 C corpus (levels L0 and L1 first)

| Project                                                              | Why                                                       | Size class |
| -------------------------------------------------------------------- | --------------------------------------------------------- | ---------- |
| Small teaching programs (K&R-style exercises, algorithm collections) | Rule coverage, fast feedback                              | tiny       |
| `cJSON`                                                              | Pure C parser, heap-heavy, pointer graph                  | small      |
| `zlib` (a subset first: `adler32`, `crc32`, then `inflate`)          | Dense bit manipulation, macros, performance-sensitive     | medium     |
| `libpng` / `lodepng`                                                 | Structured parsing, buffers                               | medium     |
| `sqlite` amalgamation (stretch)                                      | Scale and hostile C style                                 | large      |
| `lua`                                                                | Interpreter with GC, setjmp/longjmp (tests flagging path) | medium     |
| `mbedtls` / `libsodium` subset                                       | Security-critical, constant-time concerns                 | medium     |
| `Redis` data structures (`sds`, `dict`, `ziplist`)                   | Idiomatic systems C                                       | medium     |

### 3.2 C++ corpus (L1 to L2)

| Project                                | Why                           | Size class |
| -------------------------------------- | ----------------------------- | ---------- |
| Small class-hierarchy examples         | Inheritance and trait mapping | tiny       |
| `nlohmann/json` (stretch)              | Template-heavy header-only    | large      |
| `fmt` subset                           | Templates plus formatting     | medium     |
| `tinyxml2`                             | Classes, RAII, strings        | small      |
| `rapidjson`                            | Allocators, templates         | medium     |
| `Catch2`/`doctest` internals (stretch) | Macros, templates             | large      |
| `abseil` containers subset (stretch)   | Real-world generic code       | large      |

### 3.3 Known-hard corpus (expected to flag, never to silently mistranslate)

Programs deliberately using: `setjmp/longjmp`, computed goto, type punning through unions, inline assembly, signal handlers, placement new, custom allocators, `volatile` MMIO, self-modifying pointer tricks. For each, the **expected result is a specific flag**, and tests assert that recast flags it with the right code and does not produce wrong Rust.

### 3.4 Vulnerability regression corpus

Small programs reproducing classic memory bugs (use-after-free, double free, buffer overflow, uninitialized read, integer overflow in size computation). Expected results:

- Analysis finds the bug (UB finding with correct code).
- Translation behaves according to policy (reject, flag, or defined behavior).
- Rust output of non-flagged variants does not reproduce the vulnerability.

### 3.5 Corpus management

- All versions pinned by commit hash in `corpus/manifest.toml`; checksums stored.
- Each project has a `corpus/<name>/recast.toml` and `contracts.toml` maintained alongside.
- Expected-verdict manifests record the current known state (tier distribution, flagged counts); regressions beyond a tolerance fail the nightly run.
- No corpus source is committed; it is fetched by script into a cache.

---

## 4. Metrics

Tracked per run, per project, and over time:

| Metric                   | Definition                                                      | Direction    |
| ------------------------ | --------------------------------------------------------------- | ------------ |
| Translation coverage     | Functions translated at T0 to T2 / total                        | Up           |
| T0 share                 | Functions at T0 / total                                         | Up           |
| Flagged share            | Functions at T4 / total                                         | Down         |
| Unsafe density           | `unsafe` blocks per KLOC of generated Rust                      | Down         |
| Analysis precision       | Pointers classified `Proven` / total pointers                   | Up           |
| Unknown rate by analysis | `Unknown` outputs / total queries                               | Down         |
| Differential pass rate   | Passing differential tests / total (target 100% of non-flagged) | Must be 100% |
| False-accept count       | Wrong translations that passed the gate                         | Must be 0    |
| Overhead                 | Median runtime ratio on benchmarks                              | Down         |
| Macro preservation       | Use sites preserved / total                                     | Up           |
| Idiomaticity             | Clippy warning count per KLOC; share of loops converted         | Down / Up    |
| Tool performance         | Lines per second per stage; peak RSS                            | Up / Down    |
| Determinism failures     | Count                                                           | Must be 0    |

**False-accept** is the most important number. It is measured by injecting known-bad translations (mutation testing of the translator: deliberately break a rule, such as swapping a signed/unsigned cast, and confirm the verification ladder catches it). The gate's detection rate over a mutation suite MUST be tracked and gated at release (section 6).

---

## 5. Mutation testing of the verifier

A suite of **mutants** applied to generated Rust: change `<` to `<=`, drop a wrapping operation, swap argument order, remove a bounds guard, flip a tier choice, reorder destruction. The verifier must reject each mutant that changes observable behavior on at least one covered input. Report the kill rate per rung (V2, V3, V4). Mutants surviving all rungs are either equivalent mutants (reviewed and tagged) or evidence of weak verification; the latter produce new tests.

---

## 6. Release criteria

A release candidate MUST satisfy:

1. All L1 to L5, L8, L9 tests pass.
2. L7: zero contradictions of `Proven` facts across the corpus.
3. Corpus: no regression in coverage or flagged share beyond tolerance; zero false accepts; zero determinism failures.
4. Mutation kill rate at or above the target set for the milestone (document 11).
5. Documentation consistency check passes (every rule ID in code has a doc entry and golden test; every config key documented).
6. Benchmarks published for the corpus.

---

## 7. CI organization

| Pipeline | Trigger      | Contents                                                                  |
| -------- | ------------ | ------------------------------------------------------------------------- |
| Fast     | Every commit | Build, L1, L2, L3, small L4, small L9, lints, docs check                  |
| Nightly  | Scheduled    | L4 full, L5, L6 (small and medium corpus), L8, L9 corpus, mutation subset |
| Weekly   | Scheduled    | L6 full corpus, L7, L10, full mutation suite, fuzzing campaigns           |
| Release  | Tag          | Everything plus reproducible-build check and artifact signing             |

Failures in Nightly or Weekly create tracked issues automatically with the reproducer attached.

---

## 8. Bug triage classes

| Class | Definition                                                                                       | Response                                                                  |
| ----- | ------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------- |
| P0    | Wrong output accepted by the gate; unsound `Proven` fact; nondeterminism; security issue in tool | Stop the line; fix before anything else; add regression test and mutation |
| P1    | Crash, hang or wrong output that the gate catches; wrong flag decision                           | Fix next; regression test                                                 |
| P2    | Imprecision, performance regression, poor diagnostics                                            | Scheduled                                                                 |
| P3    | Cosmetic                                                                                         | Backlog                                                                   |

Every fixed bug adds a permanent regression test at the lowest layer that reproduces it.
