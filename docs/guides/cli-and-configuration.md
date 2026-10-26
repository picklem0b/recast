# 09. CLI and Configuration

This document defines the user-facing surface of recast: commands, configuration file, contracts file, exit codes and output layout. A builder MUST treat names and defaults here as the public interface.

Status: **Specified**.

---

## 1. Commands

```
recast <command> [options]
```

| Command                    | Purpose                                                                                                                                   |
| -------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------- |
| `recast init`              | Create `recast.toml` and `recast-contracts.toml` templates in the project root; detect `compile_commands.json`, languages, and toolchains |
| `recast scan`              | Read-only survey: files, include/call graph, module candidates, construct census, risky features, feasibility estimate. Never writes Rust |
| `recast plan`              | From `scan`, produce an ordered migration plan (document 08 section 7.1)                                                                  |
| `recast contracts suggest` | Propose draft contract entries from analysis, each with a confidence label                                                                |
| `recast export`            | Run the exporter (Clang LibTooling) and write `recast-ast/1` files only                                                                   |
| `recast analyze`           | Frontend plus analysis; writes TIR and fact tables, plus findings. No Rust output                                                         |
| `recast translate`         | Full pipeline up to Rust generation (frontend, analysis, translation). No verification                                                    |
| `recast verify`            | Run the verification ladder on an existing output and write the report                                                                    |
| `recast run`               | `translate` then `verify` then safety gate. The normal command                                                                            |
| `recast gate`              | Re-evaluate the safety gate on existing evidence with a different profile                                                                 |
| `recast report`            | Re-render `report.md` / SARIF from `report.json`                                                                                          |
| `recast explain <item>`    | Print the explanation record for an item (rules, facts, rejected alternatives)                                                            |
| `recast export-build`      | Generate build-system integration (document 08 section 6.3)                                                                               |
| `recast diff`              | Compare two runs (tier changes, new findings, evidence changes)                                                                           |
| `recast clean`             | Remove `recast-out/` caches                                                                                                               |
| `recast doctor`            | Check toolchains (clang, rustc, cargo, z3, sanitizer support, sandbox) and print fixes                                                    |
| `recast version`           | Version, schema versions, rule-set hash                                                                                                   |

### 1.1 Global options

```
--config <path>          recast.toml location (default: ./recast.toml)
--contracts <path>       contracts file (default: ./recast-contracts.toml)
--compdb <path>          compile_commands.json
--out <dir>              output directory (default: ./recast-out)
--profile <name>         gate profile override
--module <name>          restrict to a module (repeatable)
--jobs <n>               parallelism (default: logical CPUs); never affects output
--seed <n>               master seed (default from config; recorded)
--format <human|json>    stdout format
--quiet / --verbose      verbosity (-v, -vv)
--offline                forbid any network access (default behavior; flag exists for clarity)
--no-ai                  disable the optional AI layer (default off anyway)
```

### 1.2 Key command options

- `recast run --incremental`: reuse cached artifacts whose input hashes are unchanged.
- `recast run --fail-on <verdict>`: exit non-zero at or below the given verdict (default `REJECT`).
- `recast scan --generate-db`: observe a build to produce `compile_commands.json`.
- `recast translate --faithful-only`: skip the idiom pass.
- `recast verify --rungs V0,V1,V2`: limit rungs.
- `recast explain --format json`.

### 1.3 Exit codes

| Code | Meaning                                                                  |
| ---- | ------------------------------------------------------------------------ |
| 0    | Success; verdict ACCEPT or ACCEPT_WITH_NOTES                             |
| 1    | Gate verdict REJECT                                                      |
| 2    | Usage or configuration error                                             |
| 3    | Input error (missing or invalid compile database, source does not build) |
| 4    | Toolchain missing or incompatible (`recast doctor` explains)             |
| 5    | Run DEGRADED (internal error in some items; partial output exists)       |
| 6    | Determinism check failed                                                 |
| 70   | Internal error (bug); a crash report is written                          |

---

## 2. Configuration file: `recast.toml`

Complete reference with defaults.

```toml
schema = 1

[project]
name = "mylib"                       # crate name (snake_case derived if absent)
root = "."                           # source root
mode = "library"                     # "library" | "whole_program"
language_level = "L1"                # max level to attempt: L0 | L1 | L2 | L3 (document 01)
target = "native"                    # "native" or a target triple; must match the Rust target

[inputs]
compile_commands = "build/compile_commands.json"
include = ["src/**", "include/**"]
exclude = ["third_party/**", "test/**"]
# third-party code kept in C/C++ and accessed through shims:
keep_native = ["third_party/**"]

[[module]]                           # optional manual module grouping
name = "parser"
paths = ["src/parser/**"]

[configurations]
mode = "single"                      # "single" | "listed" | "derived"
max_atoms = 24                       # cap for derived mode
[[configurations.listed]]
id = "linux-release"
defines = { NDEBUG = "1" }

[translation]
max_tier = 2                         # 0..4; constructs needing more are flagged
prefer_index_over_rc = false
allow_arc_default = false
assume_malloc_succeeds = false
assume_utf8 = false
int_overflow = "panic"               # "panic" | "wrap"
ub_policy = "report"                 # "report" | "strict" | "reject"
naming = "preserve"                  # "preserve" | "rust"
closed_hierarchy_enum = false
exceptions = "result"                # "result" | "panic" | "flag"
idiom_pass = true
idioms = { disable = [] }            # e.g. ["IDM-006"]
allow_unsafe = false                 # permit T3 where selection reaches it

[analysis]
points_to = "andersen"               # "andersen" | "steensgaard"
constraint_budget = 5000000
solver = "z3"
solver_timeout_ms = 5000             # per query; also a logical step limit is used
lifetime_oracle = false              # cargo-check refinement loop (document 05 section 8.3)
trust_contracts = true

[verification]
profile = "standard"                 # strict | standard | permissive | audit
seed = 20261024
rungs = ["V0","V1","V2","V4"]        # overridden by profile if omitted
[verification.fuzz]
enabled = false
executions_per_target = 1000000
engine = "libfuzzer"
[verification.tests]
original_test_command = "ctest --test-dir build"
link_swap = true
timeout_s = 120
[verification.sanitizers]
original = ["address","undefined"]
rust_miri = true
[verification.bench]
enabled = false
command = "./bench.sh"
runs = 10

[gate]
profile = "standard"
fail_on = "REJECT"
[gate.custom]                        # used only when profile = "custom"
max_tier = 2
allow_t3 = false
allow_t4 = true
require_rungs = ["V0","V1","V2"]
unexercised = "note"                 # "note" | "reject"

[interop]
abi = "c"
panic = "abort"                      # "abort" | "unwind"
exports = ["mylib_*"]                # exported symbol patterns
build_system = "cmake"               # cmake | bazel | make | meson | custom
switch_prefix = "RECAST_USE_RUST_"
keep_original_headers = true

[output]
dir = "recast-out"
crate_dir = "rust"
rust_edition = "2021"
msrv = "1.85"
toolchain = "stable-pinned"          # resolved to an exact version and recorded
emit_sarif = true
attestation = false
attestation_key = ""                 # path; never generated implicitly

[ai]                                 # optional proposer layer; off by default (ADR-003)
enabled = false
```

Validation rules:

- Unknown keys are errors (typos must not silently pass).
- Any value that changes output is included in `config_hash`.
- `jobs` and verbosity are the only settings excluded from the hash.
- `target` and the Rust target triple MUST agree or the command exits with code 2.

---

## 3. Contracts file: `recast-contracts.toml`

Contracts state facts the analysis cannot derive (document 05 section 12). Full schema:

```toml
schema = 1

[[function]]
name = "parse_header"                # qualified name or USR
[function.params.buf]
ownership = "borrowed"               # borrowed | borrowed_mut | owned | consumed | retained
nullable = false
length_param = "len"                 # or length = 16 | nul_terminated = true
[function.params.out]
ownership = "borrowed_mut"
nullable = false
initialized = false                  # memory is uninitialized on entry
[function.returns]
ownership = "owned"
nullable = true
release = "free_header"
[function.behavior]
thread_safe = true
reentrant = true
may_fail_with_errno = false
pure = false

[[type]]
name = "Node"
[type.fields.next]
ownership = "owned"
nullable = true
[type.fields.parent]
ownership = "borrowed"
nullable = true
[type.invariants]
send = true
sync = false

[[allocator]]
alloc = "xmalloc"
free = "xfree"
realloc = "xrealloc"

[[callback]]
type = "handler_fn"
user_data_param = 1
lifetime = "until_unregistered"
thread = "caller"

[[macro]]
name = "CHECK"
class = "control_flow"               # override classification
[[macro]]
name = "SQ"
class = "fn_expr_pure"

[[assume]]
fact = "no_aliasing"                 # use sparingly
scope = "function:merge"
reason = "restrict semantics documented in header"
force = false
```

Rules:

- Every contract fact is `Assumed` and listed in the report.
- Missing required contract data for an exported symbol: the shim generator refuses and names the exact key needed.
- `reason` is required on `[[assume]]`.
- `recast contracts suggest` writes drafts to `recast-contracts.suggested.toml` with a comment per entry stating the evidence and confidence. Suggested files are never loaded automatically.

---

## 4. Output layout: `recast-out/`

```
recast-out/
  manifest.json                 # tool version, hashes, config, toolchains, timings
  ast/                          # recast-ast/1 per translation unit
  tir/                          # recast-ir/1, one file per module
  facts/                        # fact tables as JSON, one per analysis
  findings.json                 # all findings, stable order
  plan.json                     # from recast plan
  rust/                         # the generated crate (document 06 section 17)
  explain/                      # per-item explanation records
  verify/                       # verification artifacts (document 07 section 7)
  report/
    report.json
    report.md
    report.sarif
    attestation.json
  logs/                         # per-stage logs (not part of determinism hash)
  cache/                        # content-addressed stage cache
```

Caching: stage outputs are keyed by hash of (inputs, config subset, tool version, rule-set hash). `--incremental` reads the cache; a cache hit MUST give byte-identical output to a miss.

---

## 5. Findings format

```json
{
	"id": "F-000123",
	"code": "UB-001",
	"severity": "warning",
	"confidence": "Likely",
	"item": "fn:c:@F@add#I#I#",
	"location": { "file": "src/a.c", "line": 12, "col": 7 },
	"message": "Signed addition may overflow",
	"evidence": ["range(a) = [0, INT_MAX]", "range(b) = [1, INT_MAX]"],
	"translation": "panics on overflow (int_overflow = panic)",
	"suggestion": "Add a bound check or set int_overflow = \"wrap\" if wrapping was intended"
}
```

Severity: `error` (blocks), `warning`, `note`. Codes: `UB-nnn`, `UNSUPPORTED:<construct>`, `CONTRACT:<kind>`, `ANALYSIS:<kind>` (for example `ANALYSIS:UNSAT_OWNERSHIP`), `CONFIG:<kind>`, `TOOL:<kind>`.

---

## 6. Environment variables

| Variable           | Effect                                  |
| ------------------ | --------------------------------------- |
| `RECAST_CONFIG`    | Default config path                     |
| `RECAST_LOG`       | Log filter (`info`, `debug`, per-crate) |
| `RECAST_CACHE_DIR` | Override cache location                 |
| `NO_COLOR`         | Disable color                           |

Nothing in the environment may change translation output except `RECAST_CONFIG` selecting a different file.

---

## 7. Machine interface

`--format json` makes every command print one JSON object per line (events: `stage_started`, `stage_finished`, `finding`, `item_done`, `verdict`) so CI systems and IDE plugins can follow progress. A stable subset of this stream is part of the public API (schema `recast-events/1`).

---

## 8. Example sessions

```
$ recast init
$ recast scan                        # what's in here, how hard is it
$ recast plan                        # which module first
$ recast contracts suggest           # draft API assumptions; edit them
$ recast run --module parser         # translate + verify + gate
$ recast explain fn:parser::parse    # why was this T2?
$ recast export-build                # CMake integration with RECAST_USE_RUST_PARSER switch
```
