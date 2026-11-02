#!/usr/bin/env bash
# M2 progress: every case under corpus/m2/ translates and passes the V2
# differential. This is not the M2 exit criteria (document 11); it reports the
# sub-corpus the current build point covers.
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
failures=0

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
      --out "$out" --format json --min-kill-rate 95 >"$out/report.json"; then
    echo "FAIL $name: verify" >&2
    cat "$out/report.json" >&2 || true
    failures=$((failures + 1))
    continue
  fi

  read -r calls passed < <(python3 - "$out/report.json" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as handle:
    report = json.load(handle)
print(report["calls"][1], report["calls"][0])
PY
)
  total_calls=$((total_calls + calls))
  passed_calls=$((passed_calls + passed))
  echo "pass $name: V2 $passed/$calls"
done

if ((failures > 0)); then
  echo "$failures M2 case(s) failed" >&2
  exit 1
fi
if ((total_calls == 0)); then
  echo "no M2 cases ran" >&2
  exit 1
fi
if ((passed_calls != total_calls)); then
  echo "V2 differential passed $passed_calls of $total_calls calls" >&2
  exit 1
fi
echo "M2 corpus: $passed_calls/$total_calls calls agree across $(find corpus/m2 -mindepth 1 -maxdepth 1 -type d | wc -l) case(s)"
