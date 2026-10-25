# Crates

No Cargo manifest exists in v1.0.x. The crate boundaries, responsibilities and first files are defined in [`../docs/SCAFFOLD.md`](../docs/SCAFFOLD.md).

The planned crates are:

- `recast-schema`: versioned AST schema shared with the exporter.
- `recast-ir`: TIR types, IDs, validation and dump format.
- `recast-frontend`: schema to TIR lowering, merge, configurations and macros.
- `recast-analysis`: points-to, ownership, nullability, extent, lifetimes, escape and UB.
- `recast-translate`: tier selection, mappings, emission and the idiom pass.
- `recast-rt`: runtime support for generated code.
- `recast-verify`: verification ladder, differential harness, gate and report.
- `recast-interop`: shims, layout assertions and build integration.
- `recast-orchestrator`: pipeline, scheduler and cache.
- `recast-cli`: command surface and exit codes.
- `recast-testkit`: snippet harness, goldens and annotation parser.

The first manifests and source files are v1.1.x work. Do not create a crate directory before its first file is scheduled.
