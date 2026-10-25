//! Workspace automation.
//!
//! `xtask check-goldens` is part of the fast CI gate; `xtask bless` rewrites
//! expected output and must be reviewed like any other change.

use std::path::{Path, PathBuf};

use recast_frontend::lower;
use recast_schema::Ast;
use recast_testkit::golden::{self, Case, Outcome};
use recast_translate::translate;

/// Generate the Rust source a golden case expects.
pub fn generate(case: &Case) -> Result<String, String> {
    let ast_path = case.ast();
    let text = std::fs::read_to_string(&ast_path)
        .map_err(|error| format!("cannot read {}: {error}", ast_path.display()))?;
    let ast = Ast::from_json(&text)
        .map_err(|error| format!("{} is not recast-ast/1: {error}", ast_path.display()))?;
    let program =
        lower(&ast).map_err(|error| format!("cannot lower {}: {error}", ast_path.display()))?;
    program.validate().map_err(|errors| {
        let messages = errors
            .iter()
            .map(|error| error.to_string())
            .collect::<Vec<_>>()
            .join("; ");
        format!("TIR validation failed: {messages}")
    })?;
    translate(&program)
        .map(|translation| translation.source)
        .map_err(|error| format!("cannot translate {}: {error}", case.name))
}

/// Check every case under `root`.
pub fn check(root: &Path) -> Result<Vec<Outcome>, String> {
    golden::run(root, false, generate).map_err(|error| error.to_string())
}

/// Rewrite every expected file under `root`.
pub fn bless(root: &Path) -> Result<Vec<Outcome>, String> {
    golden::run(root, true, generate).map_err(|error| error.to_string())
}

/// Print a compact report and return whether every case passed.
pub fn report(root: &Path, outcomes: &[Outcome]) -> bool {
    let mut failures = 0;
    for outcome in outcomes {
        match &outcome.status {
            recast_testkit::golden::Status::Pass => println!("ok   {}", outcome.case),
            recast_testkit::golden::Status::Created => println!("new  {}", outcome.case),
            recast_testkit::golden::Status::Updated => println!("upd  {}", outcome.case),
            recast_testkit::golden::Status::Missing => {
                failures += 1;
                println!("fail {} (expected.rs is missing)", outcome.case);
            }
            recast_testkit::golden::Status::Mismatch { expected, actual } => {
                failures += 1;
                println!(
                    "fail {} ({})",
                    outcome.case,
                    first_difference(expected, actual)
                );
            }
        }
    }
    println!(
        "{} case(s) under {}; {} passed",
        outcomes.len(),
        root.display(),
        outcomes.len() - failures
    );
    failures == 0
}

fn first_difference(expected: &str, actual: &str) -> String {
    let expected_lines: Vec<&str> = expected.lines().collect();
    let actual_lines: Vec<&str> = actual.lines().collect();
    for (index, (expected, actual)) in expected_lines.iter().zip(&actual_lines).enumerate() {
        if expected != actual {
            return format!(
                "line {}: expected {:?}, found {:?}",
                index + 1,
                expected,
                actual
            );
        }
    }
    format!(
        "expected {} line(s), found {}",
        expected_lines.len(),
        actual_lines.len()
    )
}

/// The golden root relative to the workspace.
pub fn default_root() -> PathBuf {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/golden");
    std::fs::canonicalize(&root).unwrap_or(root)
}
