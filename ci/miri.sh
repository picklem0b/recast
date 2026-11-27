#!/usr/bin/env bash
# V4 Miri check. Needs a toolchain whose cargo provides `miri` (nightly with
# the miri and rust-src components); CI sets RUSTUP_TOOLCHAIN=nightly.
#
# The check runs one corpus case end to end and asserts that Miri, not a note,
# is the V4 evidence: the run writes `verify/v4/miri.txt`, and the report must
# carry no "cargo miri is not installed" finding.
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$root"

if ! cargo miri --version >/dev/null 2>&1; then
  echo "cargo miri is not available; install the miri and rust-src components on a nightly toolchain" >&2
  exit 4
fi

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

# Mutation is a separate criterion; this run only needs V4's Miri evidence.
"$recast" run --case "$case_dir/case.json" --source "$case_dir/source.c" \
  --compdb "$work/compile_commands.json" --exporter "$exporter" \
  --out "$work/out" --profile standard --no-mutation --quiet

python3 - "$work/out" <<'PY'
import json
import sys

out = sys.argv[1]

with open(f"{out}/verify/v4/miri.txt", encoding="utf-8") as handle:
    passed, detail = (line.strip() for line in handle.read().splitlines()[:2])
assert passed == "true", f"Miri did not pass: {detail}"
assert "Miri interpreted" in detail, f"unexpected Miri detail: {detail}"

with open(f"{out}/report/report.json", encoding="utf-8") as handle:
    report = json.load(handle)

notes = [
    finding["message"]
    for finding in report["findings"]
    if finding["rule"] == "TOOL:V4"
]
assert not any("cargo miri is not installed" in note for note in notes), notes

v4 = [
    evidence
    for item in report["items"]
    for evidence in item["evidence"]
    if evidence["rung"] == "V4"
]
assert v4, "the report has no V4 evidence"
assert all(evidence["result"] == "pass" for evidence in v4), v4

print(detail)
PY

echo "Miri check: V4 interpreted the translated calls and reported no undefined behavior"
