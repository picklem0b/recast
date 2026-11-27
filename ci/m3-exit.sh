#!/usr/bin/env bash
# M3 exit criteria (document 11): the 98% mutation threshold on the M2 corpus,
# a report that validates with its limitations statement, GATE-005, and
# `recast run` as the one-step workflow from a compile database.
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$root"

bash ci/m3-mutation.sh

exporter="$root/exporter/build/recast-exporter"
if [[ ! -x "$exporter" ]]; then
  echo "missing $exporter; build exporter/ first" >&2
  exit 4
fi

cargo build --quiet -p recast-cli
recast="$root/target/debug/recast"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

case_dir=corpus/m2/records
mkdir -p "$work/out"
python3 - "$root" "$case_dir" "$work" <<'PY'
import json
import os
import sys

root, case_dir, work = sys.argv[1], sys.argv[2], sys.argv[3]
source = os.path.abspath(os.path.join(case_dir, "source.c"))
command = {
    "directory": root,
    "file": source,
    "command": f"clang -std=c11 -c {source} -o {os.path.join(work, 'case.o')}",
}
with open(os.path.join(work, "compile_commands.json"), "w", encoding="utf-8") as handle:
    json.dump([command], handle, indent=2)
PY

# One command, no pre-existing AST: `run` exports from the compile database,
# then translates, verifies and gates. Exit 0 means the gate is not REJECT.
"$recast" run --case "$case_dir/case.json" --source "$case_dir/source.c" \
  --compdb "$work/compile_commands.json" --exporter "$exporter" \
  --out "$work/out" --profile standard --max-mutants 500 --quiet

# Re-rendering validates report.json against recast-report/1 before it writes.
"$recast" report --out "$work/out" --quiet

python3 - "$work/out" <<'PY'
import json
import sys

out = sys.argv[1]


def load(name):
    with open(f"{out}/{name}", encoding="utf-8") as handle:
        return json.load(handle)


report = load("report/report.json")
assert report["schema"] == "recast-report/1", report["schema"]
assert report["determinism"]["identical"] is True, report["determinism"]
assert report["determinism"]["runs"] == 2, report["determinism"]
assert report["verdict"] == "ACCEPT_WITH_NOTES", report["verdict"]

mutation = report["summary"]["mutation"]
assert mutation["killed"] == mutation["generated"], mutation
assert mutation["survived"] == 0, mutation
assert sum(mutation["by_rung"].values()) + mutation["killed_by_compile"] == mutation["killed"], (
    mutation
)

gate = load("verify/gate.json")
assert gate["schema"] == "recast-gate/1", gate["schema"]
assert gate["verdict"] == "ACCEPT_WITH_NOTES", gate["verdict"]
failed = [rule["id"] for rule in gate["rules"] if rule["status"] == "fail"]
assert not failed, failed

with open(f"{out}/report/report.md", encoding="utf-8") as handle:
    markdown = handle.read()
assert "## Limitations" in markdown, "report.md has no limitations section"
assert "do not prove equivalence" in markdown, "the limitations statement is missing"
assert "| Mutants rejected by the build |" in markdown, "report.md omits the build rejections"
PY

echo "M3 exit criteria pass: mutation threshold, validated report, determinism and one-step run"
