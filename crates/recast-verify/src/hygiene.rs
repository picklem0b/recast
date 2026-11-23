//! V1 static hygiene for the generated crate.
//!
//! V1 runs the configured linter, inventories every `unsafe` token, checks the
//! layout assertions emitted for `#[repr(C)]` types, and rejects a small set of
//! forbidden patterns. The checks read the generated source as text: the
//! translator emits a controlled subset, so a full syntax tree is not needed to
//! make the inventory exact.

use std::path::Path;
use std::process::Command;

use serde::Serialize;

use crate::{write, Error, Options};

/// One check with a detail line for the artifact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Check {
    pub passed: bool,
    pub detail: String,
}

impl Check {
    fn pass(detail: impl Into<String>) -> Self {
        Self {
            passed: true,
            detail: detail.into(),
        }
    }

    fn fail(detail: impl Into<String>) -> Self {
        Self {
            passed: false,
            detail: detail.into(),
        }
    }
}

/// The V1 result for one generated crate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HygieneReport {
    pub clippy: Check,
    pub unsafe_tokens: usize,
    pub forbidden: Vec<String>,
    pub repr_c_types: usize,
    pub layout_asserts: usize,
    pub notes: Vec<String>,
}

impl HygieneReport {
    /// Every V1 check passed.
    pub fn passed(&self) -> bool {
        self.clippy.passed
            && self.unsafe_tokens == 0
            && self.forbidden.is_empty()
            && self.layout_asserts >= self.repr_c_types
    }
}

/// Run V1 over the crate `crate_dir` whose library source is `source`.
pub fn run(options: &Options, crate_dir: &Path, source: &str) -> Result<HygieneReport, Error> {
    let manifest = crate_dir.join("Cargo.toml");
    let target = options.work.join("v1/target");
    std::fs::create_dir_all(&target).map_err(|error| Error::Io {
        path: target.clone(),
        message: error.to_string(),
    })?;

    // Document 07 section 3.2 asks for a deny-list, not `-D warnings`:
    // correctness, suspicious and perf lints fail V1; the style lints a
    // generated crate may legitimately trigger are allowed.
    let clippy = Command::new(&options.cargo)
        .args(["clippy", "--offline", "--quiet", "--manifest-path"])
        .arg(&manifest)
        .args([
            "--",
            "-D",
            "clippy::correctness",
            "-D",
            "clippy::suspicious",
            "-D",
            "clippy::perf",
        ])
        .env("CARGO_TARGET_DIR", &target)
        .env("CARGO_NET_OFFLINE", "true")
        .output()
        .map_err(|error| Error::Io {
            path: options.cargo.clone(),
            message: error.to_string(),
        })?;
    let clippy = if clippy.status.success() {
        Check::pass("cargo clippy passed the correctness, suspicious and perf deny-list")
    } else {
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&clippy.stdout),
            String::from_utf8_lossy(&clippy.stderr)
        );
        Check::fail(last_lines(&text, 20))
    };

    let unsafe_tokens = count_token(source, "unsafe");
    let mut forbidden = Vec::new();
    for pattern in ["static mut", "transmute", "from_raw_parts", ".unwrap()"] {
        if contains_code(source, pattern) {
            forbidden.push(pattern.to_string());
        }
    }
    let repr_c_types = source.matches("#[repr(C)]").count();
    let layout_asserts = source.matches("core::mem::size_of::<").count();
    let mut notes = Vec::new();
    if unsafe_tokens > 0 {
        notes.push(format!(
            "{unsafe_tokens} `unsafe` token(s) in a crate that forbids unsafe code"
        ));
    }
    if layout_asserts < repr_c_types {
        notes.push(format!(
            "{repr_c_types} repr(C) type(s) but only {layout_asserts} layout assertion(s)"
        ));
    }
    for pattern in &forbidden {
        notes.push(format!("forbidden pattern `{pattern}`"));
    }

    let report = HygieneReport {
        clippy,
        unsafe_tokens,
        forbidden,
        repr_c_types,
        layout_asserts,
        notes,
    };

    let v1 = options.work.join("v1");
    std::fs::create_dir_all(&v1).map_err(|error| Error::Io {
        path: v1.clone(),
        message: error.to_string(),
    })?;
    write(
        &v1.join("clippy.txt"),
        &format!(
            "{}\n{}",
            report.clippy.passed,
            report.clippy.detail.trim_end()
        ),
    )?;
    let inventory = serde_json::json!({
        "schema": "recast-unsafe-inventory/1",
        "tokens": report.unsafe_tokens,
        "blocks": [],
        "forbidden": report.forbidden,
    });
    let inventory = serde_json::to_string_pretty(&inventory)
        .map_err(|error| Error::Unsupported(error.to_string()))?;
    write(&v1.join("unsafe_inventory.json"), &format!("{inventory}\n"))?;
    let layout = serde_json::json!({
        "schema": "recast-layout-check/1",
        "repr_c_types": report.repr_c_types,
        "layout_asserts": report.layout_asserts,
        "passed": report.layout_asserts >= report.repr_c_types,
    });
    let layout = serde_json::to_string_pretty(&layout)
        .map_err(|error| Error::Unsupported(error.to_string()))?;
    write(&v1.join("layout_check.json"), &format!("{layout}\n"))?;

    Ok(report)
}

fn last_lines(text: &str, count: usize) -> String {
    let lines: Vec<&str> = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    lines[lines.len().saturating_sub(count)..].join("\n")
}

/// Count a Rust token outside comments, string literals and character literals.
pub(crate) fn count_token(source: &str, token: &str) -> usize {
    let mut count = 0;
    let mut index = 0;
    let bytes = source.as_bytes();
    while index < bytes.len() {
        if bytes[index] == b'/' && bytes.get(index + 1) == Some(&b'/') {
            index += 2;
            while index < bytes.len() && bytes[index] != b'\n' {
                index += 1;
            }
            continue;
        }
        if bytes[index] == b'/' && bytes.get(index + 1) == Some(&b'*') {
            index += 2;
            while index + 1 < bytes.len() && !(bytes[index] == b'*' && bytes[index + 1] == b'/') {
                index += 1;
            }
            index = (index + 2).min(bytes.len());
            continue;
        }
        if bytes[index] == b'"' {
            index += 1;
            while index < bytes.len() {
                if bytes[index] == b'\\' {
                    index += 2;
                } else if bytes[index] == b'"' {
                    index += 1;
                    break;
                } else {
                    index += 1;
                }
            }
            continue;
        }
        if bytes[index] == b'\'' {
            index += 1;
            if index < bytes.len() && bytes[index] == b'\\' {
                index += 2;
            } else {
                index += 1;
            }
            while index < bytes.len() && bytes[index] != b'\'' {
                index += 1;
            }
            index = (index + 1).min(bytes.len());
            continue;
        }
        if source[index..].starts_with(token) {
            let end = index + token.len();
            // `#![forbid(unsafe_code)]` is the rule, not an `unsafe` use.
            if token == "unsafe" && source[end..].starts_with("_code") {
                index = end;
                continue;
            }
            count += 1;
            index = end;
        } else {
            index += 1;
        }
    }
    count
}

fn contains_code(source: &str, pattern: &str) -> bool {
    let mut index = 0;
    let bytes = source.as_bytes();
    while index < bytes.len() {
        if bytes[index] == b'/' && bytes.get(index + 1) == Some(&b'/') {
            index += 2;
            while index < bytes.len() && bytes[index] != b'\n' {
                index += 1;
            }
            continue;
        }
        if bytes[index] == b'/' && bytes.get(index + 1) == Some(&b'*') {
            index += 2;
            while index + 1 < bytes.len() && !(bytes[index] == b'*' && bytes[index + 1] == b'/') {
                index += 1;
            }
            index = (index + 2).min(bytes.len());
            continue;
        }
        if bytes[index] == b'"' {
            index += 1;
            while index < bytes.len() {
                if bytes[index] == b'\\' {
                    index += 2;
                } else if bytes[index] == b'"' {
                    index += 1;
                    break;
                } else {
                    index += 1;
                }
            }
            continue;
        }
        if source[index..].starts_with(pattern) {
            return true;
        }
        index += 1;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_in_comments_and_strings_are_not_counted() {
        let source = r#"
// unsafe in a comment
/* unsafe too */
const S: &str = "unsafe";
unsafe { }
"#;
        assert_eq!(count_token(source, "unsafe"), 1);
    }

    #[test]
    fn forbidden_patterns_ignore_comments() {
        assert!(!contains_code("// static mut X: i32 = 0;\n", "static mut"));
        assert!(contains_code("static mut X: i32 = 0;\n", "static mut"));
    }

    #[test]
    fn layout_counts_match_the_generated_shape() {
        let source = "#[repr(C)]\npub struct P { x: i32 }\nconst _: () = {\n    assert!(core::mem::size_of::<P>() == 4);\n};\n";
        assert_eq!(source.matches("#[repr(C)]").count(), 1);
        assert_eq!(source.matches("core::mem::size_of::<").count(), 1);
    }
}
