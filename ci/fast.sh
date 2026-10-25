#!/usr/bin/env bash
# Fast pipeline: frontend, translation, generated-code build and determinism.
# Runs without Clang; ci/exporter.sh covers the exporter end to end.
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$root"

cargo build --quiet -p recast-cli
recast="$root/target/debug/recast"

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

mkdir -p "$work/out/ast"
for name in add sum checksum; do
  cp "tests/fixtures/ast/$name.json" "$work/out/ast/$name.tast"
done

"$recast" analyze --out "$work/out"
"$recast" translate --out "$work/out"
rustc --edition=2021 --crate-type=lib \
  "$work/out/rust/src/lib.rs" -o "$work/generated.rmeta"

cp "$work/out/rust/src/lib.rs" "$work/first.rs"
cp "$work/out/tir/program.tir" "$work/first.tir"
"$recast" analyze --out "$work/out" >/dev/null
"$recast" translate --out "$work/out" >/dev/null

if ! cmp -s "$work/first.tir" "$work/out/tir/program.tir"; then
  echo "determinism failure: TIR differs between two runs" >&2
  exit 6
fi
if ! cmp -s "$work/first.rs" "$work/out/rust/src/lib.rs"; then
  echo "determinism failure: generated Rust differs between two runs" >&2
  exit 6
fi

echo "fast pipeline: generated crate builds and two runs are byte-identical"
