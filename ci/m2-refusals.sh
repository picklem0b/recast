#!/usr/bin/env bash
# The refusal list in docs/audits/m2.md, checked against the real exporter: a
# construct outside the M2 pointer step must fail with a message that names the
# reason, never translate to something with different behavior.
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

checked=0
refused=0

snippet() {
  local name=$1
  mkdir -p "$work/$name"
  cat > "$work/$name/source.c"
}

# Run one snippet through the pipeline and require a failure whose message
# contains the expected fragment. The first stage that fails is reported.
expect_refusal() {
  local name=$1
  local fragment=$2
  local dir="$work/$name"
  mkdir -p "$dir/ast"
  python3 - "$root" "$dir" <<'PY'
import json, os, sys

root, directory = sys.argv[1], sys.argv[2]
source = os.path.abspath(os.path.join(directory, "source.c"))
command = {
    "directory": root,
    "file": source,
    "command": f"clang -std=c11 -c {source} -o {os.path.join(directory, 'case.o')}",
}
with open(os.path.join(directory, "compile_commands.json"), "w", encoding="utf-8") as handle:
    json.dump([command], handle, indent=2)
    handle.write("\n")
PY

  checked=$((checked + 1))
  local stage=
  if ! "$recast" export --compdb "$dir/compile_commands.json" --exporter "$exporter" \
      --out "$dir" >/dev/null 2>"$dir/err"; then
    stage=export
  elif ! "$recast" analyze --out "$dir" >/dev/null 2>"$dir/err"; then
    stage=analyze
  elif ! "$recast" translate --out "$dir" >/dev/null 2>"$dir/err"; then
    stage=translate
  elif [[ -f "$dir/case.json" ]] &&
      ! "$recast" verify --case "$dir/case.json" --source "$dir/source.c" \
        --out "$dir" --format json >/dev/null 2>"$dir/err"; then
    stage=verify
  fi

  if [[ -z "$stage" ]]; then
    echo "FAIL $name: translated or verified" >&2
    return 1
  fi
  if ! grep -qF "$fragment" "$dir/err"; then
    echo "FAIL $name: $stage refused with the wrong message" >&2
    echo "  want: $fragment" >&2
    echo "  got:  $(tail -1 "$dir/err")" >&2
    return 1
  fi
  refused=$((refused + 1))
  echo "refused $name ($stage): $(tail -1 "$dir/err" | sed 's/^recast: //')"
}

snippet return_pointer <<'C'
int *ret(int *p) { return p; }
C
expect_refusal return_pointer "a function returning a pointer needs an explicit lifetime"

snippet ptr_arith <<'C'
int walk(int *p) { return *(p + 1); }
C
expect_refusal ptr_arith "operands of + need an explicit implicit cast"

snippet ptr_compare <<'C'
int cmp(int *a, int *b) { return a < b; }
C
expect_refusal ptr_compare "operator < on a pointer needs the M2 ownership model"

snippet ptr_condition <<'C'
int c(int *p) { return p ? 1 : 0; }
C
expect_refusal ptr_condition "a pointer used as a condition needs the M2 ownership model"

snippet arrow <<'C'
struct S { int x; };
int field(struct S *s) { return s->x; }
C
expect_refusal arrow "pointer member access is outside the M1 subset"

snippet cast_pointee <<'C'
int cast(void *p) { return *(int *)p; }
C
expect_refusal cast_pointee "a pointer cast that changes the pointee needs the M2 ownership model"

snippet cast_to_int <<'C'
long as_int(int *p) { return (long)p; }
C
expect_refusal cast_to_int "a cast from a pointer is outside the M2 pointer step"

snippet null_ptr <<'C'
int zero(void) { int *p = 0; return *p; }
C
expect_refusal null_ptr "a pointer cast that changes the pointee needs the M2 ownership model"

snippet malloc_free <<'C'
#include <stdlib.h>
int *alloc(int n) {
    int *p = malloc(n * sizeof(int));
    free(p);
    return 0;
}
C
expect_refusal malloc_free "unresolved callee malloc cannot enter the M0 pipeline"

snippet mut_global <<'C'
int g;
void set(int v) { g = v; }
C
expect_refusal mut_global "mutable file-scope variables need the T1 ownership model"

# A pointer parameter whose pointee is a record translates as a signature
# (`p: &mut P`) but the V2 driver refuses to build a harness for it.
snippet driver_record <<'C'
struct P { int a; int b; };
int first(struct P *p) { return 0; }
C
cat > "$work/driver_record/case.json" <<'JSON'
{
  "schema": "recast-case/1",
  "calls": [
    {
      "function": "first",
      "args": [
        1
      ]
    }
  ]
}
JSON
expect_refusal driver_record "a pointer whose pointee is not a scalar is the next M2 step"

if ((refused != checked)); then
  echo "$((checked - refused)) construct(s) were not refused" >&2
  exit 1
fi
echo "M2 refusals: $refused of $checked constructs refused with a named reason"
