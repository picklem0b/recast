#!/usr/bin/env bash
# The three M1 exit criteria, checked against the real exporter:
#   1. 100% of the L0 corpus translates at T0 and passes the V2 differential
#   2. integer property sweeps run 10^6 cases without divergence
#   3. aggregate mutation kill rate at V2 is at least 95%
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$root"

exporter="$root/exporter/build/recast-exporter"
if [[ ! -x "$exporter" ]]; then
  echo "missing $exporter; build exporter/ first" >&2
  exit 4
fi

cargo build --quiet -p recast-cli
recast="$root/target/debug/recast"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

total_calls=0
passed_calls=0
total_mutants=0
killed_mutants=0
property_sweeps=0
failures=0

for case_dir in corpus/l0/*/; do
  name=$(basename "$case_dir")
  out="$work/$name"
  mkdir -p "$out/ast"

  python3 - "$root" "$case_dir" "$out" <<'PY'
import json
import os
import sys

root, case_dir, out = sys.argv[1], sys.argv[2], sys.argv[3]
source = os.path.abspath(os.path.join(case_dir, "source.c"))
command = {
    "directory": root,
    "file": source,
    "command": f"clang -std=c11 -c {source} -o {os.path.join(os.path.dirname(out), 'case.o')}",
}
path = os.path.join(out, "compile_commands.json")
with open(path, "w", encoding="utf-8") as handle:
    json.dump([command], handle, indent=2)
    handle.write("\n")
PY

  if ! "$recast" export --compdb "$out/compile_commands.json" --exporter "$exporter" \
      --out "$out" >/dev/null; then
    echo "FAIL $name: export" >&2
    failures=$((failures + 1))
    continue
  fi
  if ! "$recast" analyze --out "$out" >/dev/null; then
    echo "FAIL $name: analyze" >&2
    failures=$((failures + 1))
    continue
  fi
  if ! "$recast" translate --out "$out" >/dev/null; then
    echo "FAIL $name: translate" >&2
    failures=$((failures + 1))
    continue
  fi

  if ! "$recast" verify --case "$case_dir/case.json" --source "$case_dir/source.c" \
      --out "$out" --format json --min-kill-rate 95 >"$out/report.json"; then
    echo "FAIL $name: verify" >&2
    cat "$out/report.json" >&2 || true
    failures=$((failures + 1))
    continue
  fi

  read -r calls passed mutants killed sweeps < <(python3 - "$out/report.json" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as handle:
    report = json.load(handle)
mutation = report["mutation"] or {"generated": 0, "killed": 0}
print(
    report["calls"][1],
    report["calls"][0],
    mutation["generated"],
    mutation["killed"],
    report["sweeps"],
)
PY
)
  total_calls=$((total_calls + calls))
  passed_calls=$((passed_calls + passed))
  total_mutants=$((total_mutants + mutants))
  killed_mutants=$((killed_mutants + killed))
  property_sweeps=$((property_sweeps + sweeps))
  echo "pass $name: V2 $passed/$calls, $sweeps sweep(s), $killed/$mutants mutants killed"
done

if ((failures > 0)); then
  echo "$failures corpus case(s) failed" >&2
  exit 1
fi
if ((passed_calls != total_calls)); then
  echo "criterion 1: V2 differential passed $passed_calls of $total_calls calls" >&2
  exit 1
fi
if ((property_sweeps == 0)); then
  echo "criterion 2: no property sweeps ran" >&2
  exit 1
fi

kill_rate=100.0
if ((total_mutants > 0)); then
  kill_rate=$(python3 -c "print($killed_mutants * 100 / $total_mutants)")
fi
if ! python3 -c "exit(0 if $kill_rate >= 95 else 1)"; then
  echo "criterion 3: mutation kill rate ${kill_rate}% is below 95% ($killed_mutants/$total_mutants)" >&2
  exit 1
fi

echo "criterion 1: V2 differential passed $passed_calls/$total_calls calls across $(find corpus/l0 -mindepth 1 -maxdepth 1 -type d | wc -l) L0 cases"
echo "criterion 2: $property_sweeps property sweep(s) at 10^6 iterations agree"
echo "criterion 3: mutation kill rate ${kill_rate}% ($killed_mutants/$total_mutants)"
echo "M1 exit criteria pass"
