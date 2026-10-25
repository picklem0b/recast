#!/usr/bin/env bash
# Full M0 pipeline from C through the Clang exporter to Rust that compiles.
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$root"

exporter="$root/exporter/build/recast-exporter"
if [[ ! -x "$exporter" ]]; then
  echo "missing $exporter; build exporter/ first" >&2
  exit 4
fi

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

python3 - "$root" "$work" <<'PY'
import json
import os
import sys

root, work = sys.argv[1], sys.argv[2]
files = ["add", "sum", "checksum"]
commands = []
for name in files:
    source = os.path.join(root, "tests/fixtures", name + ".c")
    commands.append(
        {
            "directory": root,
            "file": source,
            "command": f"clang -std=c11 -c {source} -o {os.path.join(work, name + '.o')}",
        }
    )
with open(os.path.join(work, "compile_commands.json"), "w", encoding="utf-8") as handle:
    json.dump(commands, handle, indent=2)
    handle.write("\n")
PY

"$exporter" --compdb "$work/compile_commands.json" >"$work/ast1.json"
"$exporter" --compdb "$work/compile_commands.json" >"$work/ast2.json"
if ! cmp -s "$work/ast1.json" "$work/ast2.json"; then
  echo "determinism failure: exporter output differs between two runs" >&2
  exit 6
fi

recast="$root/target/debug/recast"
"$recast" export --compdb "$work/compile_commands.json" --exporter "$exporter" --out "$work/out"
"$recast" analyze --out "$work/out"
"$recast" translate --out "$work/out"
rustc --edition=2021 --crate-type=lib \
  "$work/out/rust/src/lib.rs" -o "$work/generated.rmeta"

"$recast" export --compdb "$work/compile_commands.json" --exporter "$exporter" --out "$work/out2" >/dev/null
"$recast" analyze --out "$work/out2" >/dev/null
"$recast" translate --out "$work/out2" >/dev/null
for artifact in rust/src/lib.rs tir/program.tir; do
  if ! cmp -s "$work/out/$artifact" "$work/out2/$artifact"; then
    echo "determinism failure: $artifact differs between two runs" >&2
    exit 6
  fi
done

echo "exporter pipeline: C to compiling Rust, byte-identical across two runs"
