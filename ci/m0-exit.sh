#!/usr/bin/env bash
# The three M0 exit criteria, checked against the real exporter.
#   1. add and sum cross C -> AST -> TIR -> Rust, compile and agree at runtime
#   2. two complete runs are byte-identical
#   3. recast-ast/1 round-trips without loss
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

python3 - "$root" "$work" <<'PY'
import json
import os
import sys

root, work = sys.argv[1], sys.argv[2]
commands = []
for name in ["add", "sum", "checksum"]:
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

for run in out out2; do
  "$recast" export --compdb "$work/compile_commands.json" --exporter "$exporter" --out "$work/$run" >/dev/null
  "$recast" analyze --out "$work/$run" >/dev/null
  "$recast" translate --out "$work/$run" >/dev/null
done

if ! cmp -s "$work/ast1.json" "$work/ast2.json"; then
  echo "criterion 2: exporter output differs between two runs" >&2
  exit 6
fi
for artifact in rust/src/lib.rs tir/program.tir; do
  if ! cmp -s "$work/out/$artifact" "$work/out2/$artifact"; then
    echo "criterion 2: $artifact differs between two runs" >&2
    exit 6
  fi
done
echo "criterion 2: two complete runs are byte-identical"

rustc --edition=2021 --crate-type=lib --crate-name recast_out \
  "$work/out/rust/src/lib.rs" -o "$work/librecast_out.rlib"
cat >"$work/rust_harness.rs" <<'RS'
fn main() {
    let a = [1, 2, 3, 4];
    println!("{} {} {}", recast_out::add(2, 3), recast_out::sum(&a), recast_out::checksum(2, 3));
}
RS
rustc --edition=2021 "$work/rust_harness.rs" \
  --extern recast_out="$work/librecast_out.rlib" -L "$work" -o "$work/rust-program"
rust_output=$("$work/rust-program")

cat >"$work/c_harness.c" <<'C'
#include <stddef.h>
#include <stdio.h>

int add(int a, int b);
int sum(const int *a, size_t n);
unsigned int checksum(unsigned int a, unsigned int b);

int main(void) {
    int a[4] = {1, 2, 3, 4};
    printf("%d %d %u\n", add(2, 3), sum(a, 4), checksum(2, 3));
    return 0;
}
C
clang -std=c11 -O0 -o "$work/c-program" \
  "$work/c_harness.c" \
  "$root/tests/fixtures/add.c" \
  "$root/tests/fixtures/sum.c" \
  "$root/tests/fixtures/checksum.c"
c_output=$("$work/c-program")

if [[ "$rust_output" != "$c_output" ]]; then
  echo "criterion 1: behavior differs: C=[$c_output] Rust=[$rust_output]" >&2
  exit 1
fi
echo "criterion 1: add, sum and checksum compile and agree with C: $c_output"

cargo test -q -p recast-schema --test round_trip
echo "criterion 3: recast-ast/1 round-trips without loss"
echo "M0 exit criteria pass"
