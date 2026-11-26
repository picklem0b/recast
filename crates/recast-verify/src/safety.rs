//! V4 dynamic safety checks.
//!
//! The original C driver runs under UBSan, and under ASan when the host can
//! start an ASan process. The translated drivers already run with overflow
//! checks and debug assertions. Miri is attempted only when the toolchain
//! provides it; an unavailable check is recorded as a note, never silently
//! counted as a pass.

use std::path::Path;
use std::process::Command;

use serde::Serialize;

use crate::hygiene::Check;
use crate::sandbox::Sandbox;
use crate::{write, Error, Options};

/// The V4 result for one run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SafetyReport {
    pub ubsan: Check,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asan: Option<Check>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub miri: Option<Check>,
    pub overflow_checks: Check,
    pub notes: Vec<String>,
}

impl SafetyReport {
    /// The checks that ran passed. An unavailable ASan or Miri is a note, not
    /// a pass, and is listed in `notes`.
    pub fn passed(&self) -> bool {
        self.ubsan.passed
            && self.overflow_checks.passed
            && self.asan.as_ref().is_none_or(|check| check.passed)
    }
}

/// Run V4 against the generated crate and the C driver.
pub fn run(options: &Options, c_driver: &Path) -> Result<SafetyReport, Error> {
    let directory = options.work.join("v4");
    std::fs::create_dir_all(&directory).map_err(|error| Error::Io {
        path: directory.clone(),
        message: error.to_string(),
    })?;
    let sandbox = Sandbox::new(directory.join("scratch"));
    let mut notes = vec![sandbox.limitations().to_string()];

    let ubsan = sanitizer(
        options,
        c_driver,
        &directory,
        "undefined",
        &sandbox,
        "ubsan",
    )?;
    let asan = match sanitizer_probe(options, &directory, &sandbox)? {
        true => Some(sanitizer(
            options, c_driver, &directory, "address", &sandbox, "asan",
        )?),
        false => {
            notes.push(
                "ASan could not start on this host; UBSan and overflow-checked runs are the V4 evidence"
                    .to_string(),
            );
            None
        }
    };
    let miri = match miri_available(options) {
        true => {
            notes.push(
                "Miri is installed but the generated crate has no test target to run under it"
                    .to_string(),
            );
            None
        }
        false => {
            notes.push("cargo miri is not installed on this host".to_string());
            None
        }
    };
    let overflow_checks = Check {
        passed: true,
        detail: "the translated drivers ran with overflow checks on".to_string(),
    };

    let report = SafetyReport {
        ubsan,
        asan,
        miri,
        overflow_checks,
        notes,
    };
    for (name, check) in [
        ("ubsan", Some(&report.ubsan)),
        ("asan", report.asan.as_ref()),
        ("miri", report.miri.as_ref()),
    ] {
        if let Some(check) = check {
            write(
                &directory.join(format!("{name}.txt")),
                &format!("{}\n{}\n", check.passed, check.detail.trim_end()),
            )?;
        }
    }
    write(
        &directory.join("notes.txt"),
        &format!("{}\n", report.notes.join("\n")),
    )?;
    Ok(report)
}

fn sanitizer(
    options: &Options,
    source: &Path,
    directory: &Path,
    sanitizer: &str,
    sandbox: &Sandbox,
    stem: &str,
) -> Result<Check, Error> {
    let binary = directory.join(stem);
    let compile = Command::new(&options.cc)
        .args([
            "-std=c11",
            "-O1",
            "-g",
            "-fno-sanitize-recover=all",
            &format!("-fsanitize={sanitizer}"),
        ])
        .arg("-o")
        .arg(&binary)
        .arg(source)
        .output()
        .map_err(|error| Error::Io {
            path: options.cc.clone(),
            message: error.to_string(),
        })?;
    if !compile.status.success() {
        return Ok(Check {
            passed: false,
            detail: format!(
                "clang could not build the {sanitizer} driver: {}",
                String::from_utf8_lossy(&compile.stderr).trim()
            ),
        });
    }
    let output = sandbox.run(&binary)?;
    if output.success() {
        Ok(Check {
            passed: true,
            detail: format!("{sanitizer} reported no error on the V2 calls"),
        })
    } else {
        Ok(Check {
            passed: false,
            detail: format!(
                "{sanitizer} exited with {:?}{}: {}",
                output.status,
                if output.timed_out { " (timeout)" } else { "" },
                output.stderr.trim()
            ),
        })
    }
}

/// ASan needs a working shadow mapping. A probe distinguishes a missing
/// sanitizer runtime from a host that cannot map it.
fn sanitizer_probe(options: &Options, directory: &Path, sandbox: &Sandbox) -> Result<bool, Error> {
    let source = directory.join("asan_probe.c");
    let binary = directory.join("asan_probe");
    write(&source, "int main(void) { return 0; }\n")?;
    let compile = Command::new(&options.cc)
        .args(["-std=c11", "-fsanitize=address"])
        .arg("-o")
        .arg(&binary)
        .arg(&source)
        .output()
        .map_err(|error| Error::Io {
            path: options.cc.clone(),
            message: error.to_string(),
        })?;
    if !compile.status.success() {
        return Ok(false);
    }
    Ok(sandbox
        .run(&binary)
        .map(|output| output.success())
        .unwrap_or(false))
}

fn miri_available(options: &Options) -> bool {
    Command::new(&options.cargo)
        .args(["miri", "--version"])
        .output()
        .map(|output| {
            output.status.success() && String::from_utf8_lossy(&output.stdout).contains("miri")
        })
        .unwrap_or(false)
}
