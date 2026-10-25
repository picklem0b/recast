# recast exporter

The exporter reads C with Clang LibTooling and writes `recast-ast/1` JSON for the Rust frontend. It never executes the input.

## Requirements

- CMake 3.20 or newer
- Ninja or Make
- LLVM and Clang development packages, version 18 or newer
- A C++17 compiler

On Ubuntu:

```sh
apt-get install cmake ninja-build clang-21 libclang-21-dev llvm-21-dev
```

## Build

```sh
cmake -S exporter -B exporter/build -G Ninja \
  -DCMAKE_BUILD_TYPE=Release \
  -DLLVM_DIR=/usr/lib/llvm-21/lib/cmake/llvm \
  -DClang_DIR=/usr/lib/llvm-21/lib/cmake/clang
cmake --build exporter/build
```

`LLVM_DIR` and `Clang_DIR` are optional when `llvm-config` on `PATH` belongs to the same installation.

## Run

```sh
exporter/build/recast-exporter --compdb build/compile_commands.json > ast.json
exporter/build/recast-exporter --compdb build/compile_commands.json --file src/add.c > add.json
```

`--compdb` names the `compile_commands.json` file. Without `--file`, every source file in the database is exported. The JSON document has one target record and one translation unit per source file.

Output is deterministic for a fixed Clang version, target and compilation database. Functions appear in source order.
