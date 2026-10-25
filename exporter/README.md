# Exporter

The exporter is a small C++ program built on Clang LibTooling. It reads one translation unit from `compile_commands.json` and emits the versioned `recast-ast/1` schema, including macro expansion records and target layout.

No C++ source or CMake file exists in v1.0.x. The first files in v1.1.x are `CMakeLists.txt`, `src/main.cpp`, `src/ast_exporter.cpp` and `src/pp_records.cpp`.

The exporter only reads and reports. It does not resolve macros, execute input code or decide translation tiers.
