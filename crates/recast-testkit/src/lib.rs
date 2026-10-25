//! Snippet harness, golden runner and shared test helpers.
//!
//! The testkit is deliberately free of stage dependencies. Callers supply the
//! function that turns a golden case into output, which keeps the runner usable
//! from `xtask`, from integration tests, and from future verification crates.

pub mod golden;
pub mod harness;

use std::fmt;
use std::path::PathBuf;

/// A testkit failure with enough context to fix it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    MissingTool { tool: &'static str },
    Io { path: PathBuf, message: String },
    Compile { tool: &'static str, message: String },
    Generate { case: String, message: String },
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::MissingTool { tool } => write!(formatter, "{tool} was not found on PATH"),
            Error::Io { path, message } => write!(formatter, "{}: {message}", path.display()),
            Error::Compile { tool, message } => {
                write!(formatter, "{tool} failed to compile the snippet: {message}")
            }
            Error::Generate { case, message } => write!(formatter, "{case}: {message}"),
        }
    }
}

impl std::error::Error for Error {}

fn io_error(path: PathBuf, error: std::io::Error) -> Error {
    Error::Io {
        path,
        message: error.to_string(),
    }
}
