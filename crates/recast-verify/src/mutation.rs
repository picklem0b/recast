//! Operator mutation testing.
//!
//! The generated Rust is mutated one operator at a time, rebuilt and re-run
//! against the C baseline captured by V2. A mutant is killed when the output
//! differs or when it no longer compiles, and survives when behavior is
//! unchanged.

use std::path::Path;

use crate::{
    compile_library, compile_rust_driver, run as run_binary, write, Case, Error, MutationReport,
    Options,
};

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
    _program: &recast_ir::Program,
    _case: &Case,
    _source: &Path,
    library: &Path,
    c_expected: &str,
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
        survived: Vec::new(),
    };

    for (index, mutant) in mutants.iter().enumerate() {
        let directory = root.join(format!("m{index:03}"));
        std::fs::create_dir_all(&directory).map_err(|error| Error::Io {
            path: directory.clone(),
            message: error.to_string(),
        })?;
        let library_path = directory.join("lib.rs");
        write(&library_path, &mutant.source)?;
        let rlib = directory.join("libgenerated.rlib");
        match compile_library(options, &library_path, &rlib) {
            Ok(()) => {}
            Err(Error::V0 { .. }) => {
                report.killed_by_compile += 1;
                continue;
            }
            Err(error) => return Err(error),
        }
        let driver = directory.join("driver");
        if compile_rust_driver(options, &options.work.join("calls.rs"), &rlib, &driver).is_err() {
            report.killed_by_compile += 1;
            continue;
        }
        // A panic, abort or hang is a behavioral difference. Only the
        // compilation steps decide whether a mutant is buildable.
        match run_binary(&driver) {
            Ok(output) if output == c_expected => {
                report.survived.push(mutant.description.clone());
            }
            Ok(_) | Err(_) => report.killed_by_behavior += 1,
        }
    }

    Ok(report)
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
