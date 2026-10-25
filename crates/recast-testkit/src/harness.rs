//! Compile and run a snippet, capturing what a differential test compares.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::{io_error, Error};

/// What a finished run produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl Run {
    pub fn success(&self) -> bool {
        self.exit_code == Some(0)
    }

    /// Compare the observable behavior M0 differential tests rely on.
    pub fn same_behavior(&self, other: &Run) -> bool {
        self.exit_code == other.exit_code && self.stdout == other.stdout
    }
}

/// Compiles and runs snippets in a private work directory.
#[derive(Debug, Clone)]
pub struct Harness {
    root: PathBuf,
    clang: PathBuf,
    rustc: PathBuf,
}

impl Harness {
    /// Create a harness. The work directory is created if needed.
    pub fn new(root: impl Into<PathBuf>) -> Result<Self, Error> {
        let root = root.into();
        std::fs::create_dir_all(&root).map_err(|error| io_error(root.clone(), error))?;
        let clang = find_tool("RECAST_CLANG", "clang")?;
        let rustc = find_tool("RECAST_RUSTC", "rustc")?;
        Ok(Self { root, clang, rustc })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Compile C11 and run it.
    pub fn run_c(&self, name: &str, source: &str) -> Result<Run, Error> {
        let stem = sanitize(name);
        let source_path = self.root.join(format!("{stem}.c"));
        let binary = self.root.join(format!("{stem}-c"));
        std::fs::write(&source_path, source)
            .map_err(|error| io_error(source_path.clone(), error))?;
        let output = Command::new(&self.clang)
            .current_dir(&self.root)
            .args(["-std=c11", "-O0", "-o"])
            .arg(&binary)
            .arg(&source_path)
            .output()
            .map_err(|error| io_error(self.clang.clone(), error))?;
        if !output.status.success() {
            return Err(Error::Compile {
                tool: "clang",
                message: String::from_utf8_lossy(&output.stderr).into_owned(),
            });
        }
        run_binary(&binary)
    }

    /// Compile Rust 2021 and run it.
    pub fn run_rust(&self, name: &str, source: &str) -> Result<Run, Error> {
        let stem = sanitize(name);
        let source_path = self.root.join(format!("{stem}.rs"));
        let binary = self.root.join(format!("{stem}-rust"));
        std::fs::write(&source_path, source)
            .map_err(|error| io_error(source_path.clone(), error))?;
        let output = Command::new(&self.rustc)
            .current_dir(&self.root)
            .args(["--edition=2021", "-O", "-o"])
            .arg(&binary)
            .arg(&source_path)
            .output()
            .map_err(|error| io_error(self.rustc.clone(), error))?;
        if !output.status.success() {
            return Err(Error::Compile {
                tool: "rustc",
                message: String::from_utf8_lossy(&output.stderr).into_owned(),
            });
        }
        run_binary(&binary)
    }

    /// Compile C and Rust versions of one snippet and check they agree.
    pub fn differential(
        &self,
        name: &str,
        c_source: &str,
        rust_source: &str,
    ) -> Result<(Run, Run), Error> {
        let c = self.run_c(name, c_source)?;
        let rust = self.run_rust(name, rust_source)?;
        Ok((c, rust))
    }
}

fn run_binary(binary: &Path) -> Result<Run, Error> {
    let output = Command::new(binary)
        .current_dir(binary.parent().unwrap_or_else(|| Path::new(".")))
        .output()
        .map_err(|error| io_error(binary.to_path_buf(), error))?;
    Ok(Run {
        exit_code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

fn find_tool(environment: &str, name: &'static str) -> Result<PathBuf, Error> {
    if let Some(path) = std::env::var_os(environment) {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Ok(path);
        }
    }
    let path = std::env::var_os("PATH").ok_or(Error::MissingTool { tool: name })?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    Err(Error::MissingTool { tool: name })
}

fn sanitize(name: &str) -> String {
    let mut stem = name
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
        .collect::<String>();
    if stem.is_empty() {
        stem.push_str("snippet");
    }
    stem
}

#[cfg(test)]
mod tests {
    use super::*;

    fn harness(name: &str) -> Harness {
        let dir =
            std::env::temp_dir().join(format!("recast-harness-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        Harness::new(dir).expect("toolchain is available")
    }

    #[test]
    fn c_and_rust_observations_compare_by_behavior() {
        let harness = harness("behavior");
        let (c, rust) = harness
            .differential(
                "hello",
                "#include <stdio.h>\nint main(void) { printf(\"ok\\n\"); return 3; }\n",
                "fn main() { println!(\"ok\"); std::process::exit(3); }\n",
            )
            .expect("snippets compile and run");
        assert_eq!(c.exit_code, Some(3));
        assert_eq!(c.stdout, "ok\n");
        assert!(c.same_behavior(&rust), "{c:?} vs {rust:?}");
        let _ = std::fs::remove_dir_all(harness.root());
    }

    #[test]
    fn a_compile_error_is_reported_with_the_tool() {
        let harness = harness("compile-error");
        let error = harness
            .run_c("broken", "int main(void) { return missing; }\n")
            .expect_err("invalid C does not compile");
        assert!(
            matches!(error, Error::Compile { tool: "clang", .. }),
            "{error}"
        );
        let _ = std::fs::remove_dir_all(harness.root());
    }
}
