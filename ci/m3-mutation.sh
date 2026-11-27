#!/usr/bin/env bash
# M3 exit criteria, mutation half (document 11): the operator-mutation kill
# rate on the M2 corpus is at or above 98% across V2 to V4, and every survivor
# is printed for review.
#
# Every mutant is rebuilt and run against each input set the ladder executed:
# the V2 calls, the V2 property sweeps and the V3 fuzz driver, each compared
# with the C baseline that produced it. Every mutant crate is built with
# overflow checks and debug assertions on, so a panic or a hang is also a V4
# checked kill; those are counted separately as `killed_by_check`. Mutants
# that fail to build are counted as killed by compilation.
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$root"

threshold=98
cap=500

exporter="$root/exporter/build/recast-exporter"
if [[ ! -x "$exporter" ]]; then
  echo "missing $exporter; build exporter/ first" >&2
  exit 4
fi

cargo build --quiet -p recast-cli
recast="$root/target/debug/recast"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

total_generated=0
total_killed=0
total_checked=0
total_compile=0
failures=0
rungs="$work/rungs.tsv"
: >"$rungs"

for case_dir in corpus/m2/*/; do
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
with open(os.path.join(out, "compile_commands.json"), "w", encoding="utf-8") as handle:
    json.dump([command], handle, indent=2)
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
      --out "$out" --format json --max-mutants "$cap" \
      --min-kill-rate "$threshold" >"$out/report.json"; then
    echo "FAIL $name: mutation kill rate below ${threshold}% or a rung failed" >&2
    cat "$out/report.json" >&2 || true
    failures=$((failures + 1))
    continue
  fi

  read -r generated killed compile checked rungs_text < <(python3 - "$out/report.json" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as handle:
    mutation = json.load(handle)["mutation"]
per_rung = mutation.get("by_rung", {})
print(
    mutation["generated"],
    mutation["killed"],
    mutation.get("killed_by_compile", 0),
    mutation.get("killed_by_check", 0),
    ",".join(f"{rung}={count}" for rung, count in sorted(per_rung.items())),
)
PY
)
  total_generated=$((total_generated + generated))
  total_killed=$((total_killed + killed))
  total_compile=$((total_compile + compile))
  total_checked=$((total_checked + checked))
  echo "pass $name: $killed/$generated mutants killed ($rungs_text; $compile build rejection(s), $checked by a checked panic)"
  if [[ -n "$rungs_text" ]]; then
    tr ',' '\n' <<<"$rungs_text" | tr '=' '\t' >>"$rungs"
  fi
done

if ((failures > 0)); then
  echo "$failures M2 case(s) failed the M3 mutation criteria" >&2
  exit 1
fi
if ((total_generated == 0)); then
  echo "no mutants were generated on the M2 corpus" >&2
  exit 1
fi

rate=$(python3 -c "print(f'{$total_killed * 100 / $total_generated:.1f}')")
echo -n "M3 mutation: $total_killed/$total_generated killed (${rate}%) across V2 to V4 ($total_compile build rejection(s), $total_checked by a checked panic)"
if [[ -s "$rungs" ]]; then
  echo -n "; "
  awk -F'\t' '{ total[$1] += $2 } END {
    n = split("v2-calls v2-sweeps v3-fuzz", order, " ")
    for (i = 1; i <= n; i++) {
      if (order[i] in total) printf "%s%s %d", (i > 1 ? ", " : ""), order[i], total[order[i]]
    }
  }' "$rungs"
fi
echo

if ! awk -v rate="$rate" -v threshold="$threshold" \
    'BEGIN { exit !(rate + 0 >= threshold + 0) }'; then
  echo "mutation kill rate ${rate}% is below the M3 threshold ${threshold}%" >&2
  exit 1
fi
