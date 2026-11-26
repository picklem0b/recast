#!/usr/bin/env bash
# C++ input is not translated at this build point: the C++ core is milestone
# M6 (docs/project/roadmap.md). Each snippet below is valid C++ whose meaning
# changes if it is read as C, so the exporter must refuse it by name and write
# no translation. A partial AST would translate and verify as if the refused
# unit did not exist, so a mixed database is refused as a whole.
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

# A directory per case: one source file, one compile_commands.json.
case_dir() {
  local name=$1
  mkdir -p "$work/$name"
  echo "$work/$name"
}

compdb() {
  local dir=$1
  local std=$2
  shift 2
  python3 - "$dir" "$std" "$@" <<'PY'
import json, os, sys

directory, std, *names = sys.argv[1:]


def compiler_for(std):
    # clang++ is clang with -x c++. Naming the language explicitly keeps the
    # check independent of whether the host ships the clang++ driver.
    return "clang" if std == "c11" else "clang -x c++"
commands = []
for name in names:
    source = os.path.join(directory, name)
    commands.append(
        {
            "directory": directory,
            "file": source,
            "command": compiler_for(std) + f" -std={std} -c {source} -o {source}.o",
        }
    )
with open(os.path.join(directory, "compile_commands.json"), "w", encoding="utf-8") as handle:
    json.dump(commands, handle, indent=2)
    handle.write("\n")
PY
}

# Run export and require exit 3, the refusal message, and no written AST.
expect_refusal() {
  local name=$1
  local dir=$2
  checked=$((checked + 1))
  local status=0
  "$recast" export --compdb "$dir/compile_commands.json" --exporter "$exporter" \
    --out "$dir/out" >/dev/null 2>"$dir/err" || status=$?
  if [[ $status -ne 3 ]]; then
    echo "FAIL $name: exit $status, wanted 3" >&2
    tail -1 "$dir/err" >&2 || true
    return 1
  fi
  if ! grep -qF "C++ input is not translated yet" "$dir/err"; then
    echo "FAIL $name: refused without naming the reason" >&2
    echo "  got: $(tail -1 "$dir/err")" >&2
    return 1
  fi
  if compgen -G "$dir/out/ast/*.tast" >/dev/null; then
    echo "FAIL $name: a refused unit still wrote an AST" >&2
    return 1
  fi
  refused=$((refused + 1))
  echo "refused $name: $(tail -1 "$dir/err" | sed 's/^recast: recast-exporter failed: recast-exporter: //')"
}

# A reference parameter: as C the write-back would be lost.
dir=$(case_dir reference)
cat >"$dir/source.cpp" <<'CPP'
int bump_ref(int &r) {
    r += 1;
    return r;
}
CPP
compdb "$dir" c++17 source.cpp
expect_refusal reference "$dir"

# A class with a constructor and a destructor: state and lifetime in C++.
dir=$(case_dir class)
cat >"$dir/source.cpp" <<'CPP'
class Counter {
public:
    explicit Counter(int start) : value_(start) {}
    ~Counter() { value_ = 0; }
    int bump() { return ++value_; }

private:
    int value_;
};

int count_to(int limit) {
    Counter counter(0);
    while (counter.bump() < limit) {
    }
    return limit;
}
CPP
compdb "$dir" c++17 source.cpp
expect_refusal class "$dir"

# Overloads share a name at C level; translating one silently would pick a
# different function for a call site.
dir=$(case_dir overloads)
cat >"$dir/source.cpp" <<'CPP'
int twice(int v) { return v * 2; }
double twice(double v) { return v * 2.0; }
CPP
compdb "$dir" c++17 source.cpp
expect_refusal overloads "$dir"

# A function template is not a function until it is instantiated.
dir=$(case_dir template)
cat >"$dir/source.cpp" <<'CPP'
template <typename T>
T twice(T v) {
    return v + v;
}

int use_template() { return twice(21); }
CPP
compdb "$dir" c++17 source.cpp
expect_refusal template "$dir"

# A database with both languages: the C unit must not be exported alone.
dir=$(case_dir mixed)
cat >"$dir/plain.c" <<'C'
int add(int a, int b) { return a + b; }
C
cat >"$dir/extra.cpp" <<'CPP'
int bump_ref(int &r) { return ++r; }
CPP
python3 - "$dir" <<'PY'
import json, os, sys

directory = sys.argv[1]
commands = []
for name, std in (("plain.c", "c11"), ("extra.cpp", "c++17")):
    source = os.path.join(directory, name)
    compiler = "clang" if std == "c11" else "clang -x c++"
    commands.append(
        {
            "directory": directory,
            "file": source,
            "command": f"{compiler} -std={std} -c {source} -o {source}.o",
        }
    )
with open(os.path.join(directory, "compile_commands.json"), "w", encoding="utf-8") as handle:
    json.dump(commands, handle, indent=2)
    handle.write("\n")
PY
expect_refusal mixed "$dir"

if ((refused != checked)); then
  echo "$((checked - refused)) C++ case(s) were not refused" >&2
  exit 1
fi
echo "C++ refusals: $refused of $checked inputs refused with a named reason"
