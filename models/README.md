# Models

Models describe the ownership, nullability, extent and side effects of external functions the analysis cannot inspect.

The first files in a later milestone are `libc.toml`, `posix.toml` and `cxx-std.toml`. A missing model means unknown behavior, which forces conservative handling; it never means a guessed model.

No model file exists in v1.0.x.
