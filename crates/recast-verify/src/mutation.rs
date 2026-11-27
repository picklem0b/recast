//! Operator mutation testing.
//!
//! The generated Rust is mutated one operator at a time and rebuilt against
//! every input set the ladder executed: the V2 calls, the V2 property sweeps
//! and the V3 fuzz driver, each compared with the output its C original
//! produced. A mutant is killed when an output differs, when the checked build
//! panics, or when it no longer compiles; it survives only when every covered
//! input agrees with C.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::{
    compile_library, compile_rust_driver, run as run_binary, write, Error, MutationReport, Options,
};

/// One driver the ladder executed, and the output its C original produced.
pub(crate) struct Input {
    /// The rung that owns the inputs: `v2-calls`, `v2-sweeps` or `v3-fuzz`.
    pub rung: &'static str,
    pub driver: PathBuf,
    pub expected: String,
}

// Comparisons flip direction rather than moving to the adjacent operator:
// `x < 0 ? -x : x` makes `<` -> `<=` an equivalent mutant at zero, which no
// input can kill. Direction changes are observable.
const OPERATORS: &[(&str, &str)] = &[
    (" + ", " - "),
    (" - ", " + "),
    (" * ", " / "),
    (" / ", " * "),
    (" % ", " / "),
    (" < ", " > "),
    (" <= ", " >= "),
    (" > ", " < "),
    (" >= ", " <= "),
    (" == ", " != "),
    (" != ", " == "),
    (" && ", " || "),
    (" || ", " && "),
    (".wrapping_add(", ".wrapping_sub("),
    (".wrapping_sub(", ".wrapping_add("),
    (".wrapping_mul(", ".wrapping_add("),
    ("checked_shl(", "checked_shr("),
];

struct Mutant {
    source: String,
    description: String,
}

pub(crate) fn run_mutation(
    options: &Options,
    library: &Path,
    inputs: &[Input],
) -> Result<MutationReport, Error> {
    let text = std::fs::read_to_string(library).map_err(|error| Error::Io {
        path: library.to_path_buf(),
        message: error.to_string(),
    })?;
    let mutants = generate(&text, options.max_mutants);
    let root = options.work.join("mutants");
    std::fs::create_dir_all(&root).map_err(|error| Error::Io {
        path: root.clone(),
        message: error.to_string(),
    })?;

    let mut report = MutationReport {
        generated: mutants.len(),
        killed_by_behavior: 0,
        killed_by_compile: 0,
        // Every rung that supplied inputs. The keys are present even when
        // nothing was killed, so a zero is visible rather than mistaken for a
        // missing measurement.
        by_rung: inputs
            .iter()
            .map(|input| (input.rung.to_string(), 0))
            .collect::<BTreeMap<_, _>>(),
        killed_by_check: 0,
        survived: Vec::new(),
    };
    // A survivor makes the report; a kill makes the log. Recording the rung,
    // the cause and the mutation behind every kill lets a reviewer check the
    // attribution without re-running the suite. A mutant the build rejects has
    // `compile` in the rung column and `build` as the cause.
    let mut kills = String::from("rung\tcause\tmutation\n");

    for (index, mutant) in mutants.iter().enumerate() {
        let directory = root.join(format!("m{index:03}"));
        std::fs::create_dir_all(&directory).map_err(|error| Error::Io {
            path: directory.clone(),
            message: error.to_string(),
        })?;
        let library_path = directory.join("lib.rs");
        write(&library_path, &mutant.source)?;
        let rlib = directory.join("libgenerated.rlib");
        if let Err(error) = compile_library(options, &library_path, &rlib) {
            match error {
                Error::V0 { .. } => {
                    report.killed_by_compile += 1;
                    // A mutant the build rejects is still a kill, and the log
                    // says so rather than leaving it unaccounted for.
                    kills.push_str(&format!("compile\tbuild\t{}\n", mutant.description));
                    continue;
                }
                other => return Err(other),
            }
        }

        // The first set that disagrees kills the mutant; the order follows the
        // ladder, so a kill is attributed to the earliest rung that saw it.
        // Every mutant crate is built with overflow checks and debug
        // assertions on, so a panic or a hang is a checked kill as well as a
        // behavioral one; the cause is recorded separately for review.
        let mut killed_by: Option<&str> = None;
        let mut by_check = false;
        for (position, input) in inputs.iter().enumerate() {
            let driver = directory.join(format!("driver-{position}"));
            if compile_rust_driver(options, &input.driver, &rlib, &driver).is_err() {
                killed_by = Some("compile");
                break;
            }
            match run_binary(&driver) {
                Ok(output) if output == input.expected => {}
                Ok(_) => {
                    killed_by = Some(input.rung);
                    break;
                }
                Err(_) => {
                    killed_by = Some(input.rung);
                    by_check = true;
                    break;
                }
            }
        }

        match killed_by {
            Some("compile") => {
                report.killed_by_compile += 1;
                kills.push_str(&format!("compile\t{}\n", mutant.description));
            }
            Some(rung) => {
                report.killed_by_behavior += 1;
                *report.by_rung.entry(rung.to_string()).or_insert(0) += 1;
                if by_check {
                    report.killed_by_check += 1;
                }
                let cause = if by_check { "panic" } else { "value" };
                kills.push_str(&format!("{rung}\t{cause}\t{}\n", mutant.description));
            }
            None => report.survived.push(mutant.description.clone()),
        }
    }

    write(&root.join("kills.tsv"), &kills)?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A mutant that the V2 calls agree with and the V3 inputs catch must be
    /// attributed to `v3-fuzz`, not to the first rung that ran.
    #[test]
    fn a_kill_is_attributed_to_the_input_set_that_saw_it() {
        let work = std::env::temp_dir().join(format!(
            "recast-mutation-attribution-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&work);
        std::fs::create_dir_all(&work).expect("work directory");

        let library = work.join("lib.rs");
        std::fs::write(&library, "pub fn f(x: i32) -> i32 {\n    x + 1\n}\n").expect("library");
        let constant = work.join("calls.rs");
        std::fs::write(&constant, "fn main() {\n    println!(\"constant\");\n}\n")
            .expect("calls driver");
        let probe = work.join("fuzz.rs");
        std::fs::write(
            &probe,
            "fn main() {\n    println!(\"{}\", generated::f(0));\n}\n",
        )
        .expect("fuzz driver");

        let options = Options::new(&work);
        let inputs = vec![
            Input {
                rung: "v2-calls",
                driver: constant,
                expected: "constant\n".to_string(),
            },
            Input {
                rung: "v3-fuzz",
                driver: probe,
                expected: "1\n".to_string(),
            },
        ];
        let report = run_mutation(&options, &library, &inputs).expect("mutation run");
        assert_eq!(report.generated, 1);
        assert_eq!(report.killed_by_behavior, 1);
        assert_eq!(report.killed_by_check, 0);
        assert!(report.survived.is_empty());
        assert_eq!(report.rungs(), vec![("v2-calls", 0), ("v3-fuzz", 1)]);
        let log = std::fs::read_to_string(work.join("mutants/kills.tsv")).expect("kill log");
        assert_eq!(
            log,
            "rung\tcause\tmutation\nv3-fuzz\tvalue\t`+` -> `-` at byte 31\n"
        );

        let _ = std::fs::remove_dir_all(&work);
    }
}

#[derive(Debug, Clone, Copy)]
enum State {
    Code,
    String,
    Char,
    LineComment,
    BlockComment,
}

/// Build mutants by replacing one operator token at a time, skipping comments
/// and literals.
fn generate(text: &str, cap: usize) -> Vec<Mutant> {
    let bytes = text.as_bytes();
    let mut mutants = Vec::new();
    let mut index = 0;
    let mut state = State::Code;
    while index < bytes.len() && mutants.len() < cap {
        match state {
            State::Code => {
                if bytes[index] == b'/' && bytes.get(index + 1) == Some(&b'/') {
                    state = State::LineComment;
                    index += 2;
                    continue;
                }
                if bytes[index] == b'/' && bytes.get(index + 1) == Some(&b'*') {
                    state = State::BlockComment;
                    index += 2;
                    continue;
                }
                if bytes[index] == b'"' {
                    state = State::String;
                    index += 1;
                    continue;
                }
                if bytes[index] == b'\'' {
                    state = State::Char;
                    index += 1;
                    continue;
                }
                if let Some((from, to)) = OPERATORS
                    .iter()
                    .find(|(from, _)| text[index..].starts_with(from))
                {
                    let mut source = String::with_capacity(text.len() + to.len());
                    source.push_str(&text[..index]);
                    source.push_str(to);
                    source.push_str(&text[index + from.len()..]);
                    mutants.push(Mutant {
                        source,
                        description: format!(
                            "`{}` -> `{}` at byte {index}",
                            from.trim(),
                            to.trim()
                        ),
                    });
                    index += from.len();
                    continue;
                }
                index += 1;
            }
            State::String => {
                if bytes[index] == b'\\' {
                    index += 2;
                } else if bytes[index] == b'"' {
                    state = State::Code;
                    index += 1;
                } else {
                    index += 1;
                }
            }
            State::Char => {
                if bytes[index] == b'\\' {
                    index += 2;
                } else if bytes[index] == b'\'' {
                    state = State::Code;
                    index += 1;
                } else {
                    index += 1;
                }
            }
            State::LineComment => {
                if bytes[index] == b'\n' {
                    state = State::Code;
                }
                index += 1;
            }
            State::BlockComment => {
                if bytes[index] == b'*' && bytes.get(index + 1) == Some(&b'/') {
                    state = State::Code;
                    index += 2;
                } else {
                    index += 1;
                }
            }
        }
    }
    mutants
}
