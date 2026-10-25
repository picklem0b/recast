# Tests

Unit tests live beside the code they cover. Integration tests live in each crate's `tests/` directory.

Cross-crate translation goldens live here:

```
tests/golden/<rule-id>/<case>/
  input.c
  config.toml
  expected.rs
  expected.explain.json
```

Every translation rule gets one positive case, one negative case and one boundary case. The scaffold creates no golden until the first rule exists in v1.1.x.
