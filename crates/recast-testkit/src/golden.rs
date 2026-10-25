//! Golden case discovery and comparison.
//!
//! A case is a directory containing `input.c`. The expected output lives next
//! to it in `expected.rs`. Case names are paths relative to the golden root,
//! sorted lexicographically, so reports and bless runs are deterministic.

use std::path::{Path, PathBuf};

use crate::{io_error, Error};

/// One golden case.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Case {
    pub name: String,
    pub dir: PathBuf,
}

impl Case {
    pub fn input(&self) -> PathBuf {
        self.dir.join("input.c")
    }

    pub fn expected(&self) -> PathBuf {
        self.dir.join("expected.rs")
    }

    /// The exported AST a case carries until the exporter runs in CI.
    pub fn ast(&self) -> PathBuf {
        self.dir.join("ast.json")
    }
}

/// What happened to one case.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    Pass,
    Created,
    Updated,
    Missing,
    Mismatch { expected: String, actual: String },
}

impl Status {
    pub fn ok(&self) -> bool {
        matches!(self, Status::Pass | Status::Created | Status::Updated)
    }
}

/// The result of running one case.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub case: String,
    pub status: Status,
}

/// Find every case under `root` in deterministic order.
pub fn discover(root: &Path) -> Result<Vec<Case>, Error> {
    let mut cases = Vec::new();
    visit(root, root, &mut cases)?;
    cases.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(cases)
}

fn visit(root: &Path, dir: &Path, cases: &mut Vec<Case>) -> Result<(), Error> {
    let mut entries = std::fs::read_dir(dir)
        .map_err(|error| io_error(dir.to_path_buf(), error))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| io_error(dir.to_path_buf(), error))?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        if path.join("input.c").is_file() {
            let name = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            cases.push(Case { name, dir: path });
        } else {
            visit(root, &path, cases)?;
        }
    }
    Ok(())
}

/// Run every case, writing expected files when `bless` is set.
///
/// `generate` receives the case and returns the output the runner compares.
pub fn run<F>(root: &Path, bless: bool, generate: F) -> Result<Vec<Outcome>, Error>
where
    F: Fn(&Case) -> Result<String, String>,
{
    let cases = discover(root)?;
    let mut outcomes = Vec::with_capacity(cases.len());
    for case in cases {
        let actual = generate(&case).map_err(|message| Error::Generate {
            case: case.name.clone(),
            message,
        })?;
        let expected_path = case.expected();
        let expected = match std::fs::read_to_string(&expected_path) {
            Ok(text) => Some(text),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(io_error(expected_path.clone(), error)),
        };

        let status = match expected {
            Some(expected) if expected == actual => Status::Pass,
            Some(expected) => {
                if bless {
                    std::fs::write(&expected_path, actual)
                        .map_err(|error| io_error(expected_path.clone(), error))?;
                    Status::Updated
                } else {
                    Status::Mismatch { expected, actual }
                }
            }
            None => {
                if bless {
                    std::fs::write(&expected_path, actual)
                        .map_err(|error| io_error(expected_path.clone(), error))?;
                    Status::Created
                } else {
                    Status::Missing
                }
            }
        };
        outcomes.push(Outcome {
            case: case.name,
            status,
        });
    }
    Ok(outcomes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("recast-golden-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("m0/one")).expect("create case");
        std::fs::write(dir.join("m0/one/input.c"), "int main(void) { return 0; }\n")
            .expect("write input");
        dir
    }

    #[test]
    fn discovery_is_sorted_and_requires_input() {
        let dir = root("discover");
        std::fs::create_dir_all(dir.join("m0/ignored")).expect("create ignored");
        std::fs::write(dir.join("m0/ignored/expected.rs"), "").expect("write expected");
        let cases = discover(&dir).expect("discover cases");
        assert_eq!(cases.len(), 1);
        assert_eq!(cases[0].name, "m0/one");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn check_reports_a_missing_expectation_and_bless_creates_it() {
        let dir = root("bless");
        let cases = discover(&dir).expect("discover cases");
        let checked =
            run(&dir, false, |case| Ok(format!("// {}\n", case.name))).expect("run check");
        assert_eq!(checked[0].status, Status::Missing);
        let blessed = run(&dir, true, |case| Ok(format!("// {}\n", case.name))).expect("run bless");
        assert_eq!(blessed[0].status, Status::Created);
        assert_eq!(
            std::fs::read_to_string(cases[0].expected()).expect("read expected"),
            "// m0/one\n"
        );
        let again = run(&dir, false, |case| Ok(format!("// {}\n", case.name))).expect("re-check");
        assert_eq!(again[0].status, Status::Pass);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
