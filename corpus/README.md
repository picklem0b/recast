# Corpus

The corpus contains pinned external C and C++ projects used for end-to-end translation tests. Source code is fetched into a cache and is never committed.

The first files in a later milestone are `manifest.toml` with pinned commits and checksums, then a per-project `recast.toml` and `contracts.toml`. The scaffold does not create them.
