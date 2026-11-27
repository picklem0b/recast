//! Verification ladder for the M1 subset.
//!
//! - **V0**: the generated crate compiles.
//! - **V1**: the original project's tests still pass (optional per case).
//! - **V2**: generated code is run against the C original on the same inputs
//!   and must produce identical output.
//!
//! The corpus runner also performs a deterministic property sweep (a shared
//! LCG on both sides, one million iterations by default) and operator mutation
//! testing against the V2 differential.

mod driver;
pub mod fuzz;
pub mod hygiene;
mod mutation;
pub mod safety;
pub mod sandbox;

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use recast_ir::Program;
use serde::Deserialize;

/// One call the differential executes on both implementations.
#[derive(Debug, Clone, Deserialize, serde::Serialize)]
pub struct Call {
    pub function: String,
    #[serde(default)]
    pub args: Vec<serde_json::Value>,
}

/// The seed shared by both sides of every property sweep.
pub const SWEEP_SEED: u64 = 0x243F_6A88_85A3_08D3;

/// A deterministic random sweep over one function.
#[derive(Debug, Clone, Deserialize)]
pub struct Sweep {
    pub function: String,
    #[serde(default = "default_iterations")]
    pub iterations: u64,
}

fn default_iterations() -> u64 {
    1_000_000
}

/// A corpus case description.
#[derive(Debug, Clone, Deserialize)]
pub struct Case {
    #[serde(default = "default_schema")]
    pub schema: String,
    #[serde(default)]
    pub calls: Vec<Call>,
    #[serde(default)]
    pub sweeps: Vec<Sweep>,
    /// An optional V1 command run in the case directory.
    #[serde(default)]
    pub test_command: Option<String>,
}

fn default_schema() -> String {
    "recast-case/1".to_string()
}

impl Case {
    /// Read a case description from JSON.
    pub fn load(path: &Path) -> Result<Self, Error> {
        let text =
            std::fs::read_to_string(path).map_err(|error| Error::io(path.to_path_buf(), error))?;
        let case: Self = serde_json::from_str(&text).map_err(|error| Error::Json {
            path: path.to_path_buf(),
            message: error.to_string(),
        })?;
        if case.schema != "recast-case/1" {
            return Err(Error::Case(format!(
                "{}: expected recast-case/1, found {}",
                path.display(),
                case.schema
            )));
        }
        Ok(case)
    }
}

/// Options for one verification run.
#[derive(Debug, Clone)]
pub struct Options {
    pub work: PathBuf,
    pub cc: PathBuf,
    pub rustc: PathBuf,
    pub cargo: PathBuf,
    /// Run V1 static hygiene on the generated crate.
    pub hygiene: bool,
    /// Run V3 differential fuzzing with this many calls.
    pub fuzz_executions: u64,
    /// Run V4 sanitizer and Miri checks.
    pub sanitizers: bool,
    /// Run operator mutation testing after the differential.
    pub mutation: bool,
    /// Maximum mutants generated per case.
    pub max_mutants: usize,
}

impl Options {
    /// Defaults for a corpus run.
    pub fn new(work: impl Into<PathBuf>) -> Self {
        Self {
            work: work.into(),
            cc: PathBuf::from("clang"),
            rustc: PathBuf::from("rustc"),
            cargo: PathBuf::from("cargo"),
            hygiene: true,
            fuzz_executions: 1_000,
            sanitizers: true,
            mutation: false,
            max_mutants: 24,
        }
    }
}

/// How a mutant was eliminated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MutationReport {
    pub generated: usize,
    /// Killed by a wrong value, a panic or a timeout on a covered input.
    pub killed_by_behavior: usize,
    pub killed_by_compile: usize,
    /// Which input set caught each mutant: `v2-calls`, `v2-sweeps` or
    /// `v3-fuzz`. Every mutant run is built with overflow checks and debug
    /// assertions on, so these are also V4's checks; a wrong value counts the
    /// same as a checked panic.
    pub by_rung: BTreeMap<String, usize>,
    /// How many of the behavioral kills were a panic or a timeout rather than
    /// a different value.
    pub killed_by_check: usize,
    pub survived: Vec<String>,
}

impl MutationReport {
    pub fn killed(&self) -> usize {
        self.killed_by_behavior + self.killed_by_compile
    }

    /// The per-rung counts as a sorted list, so a caller can print them in a
    /// fixed order.
    pub fn rungs(&self) -> Vec<(&str, usize)> {
        const ORDER: [&str; 3] = ["v2-calls", "v2-sweeps", "v3-fuzz"];
        ORDER
            .iter()
            .filter_map(|rung| self.by_rung.get(*rung).map(|count| (*rung, *count)))
            .collect()
    }

    /// Kill rate in percent, including mutants that fail to compile.
    pub fn kill_rate(&self) -> f64 {
        if self.generated == 0 {
            100.0
        } else {
            (self.killed() as f64) * 100.0 / (self.generated as f64)
        }
    }
}

/// Result of verifying one corpus case.
#[derive(Debug, Clone)]
pub struct Report {
    pub case: String,
    pub compiled: bool,
    pub calls_passed: usize,
    pub calls_total: usize,
    pub sweeps: Vec<SweepReport>,
    pub hygiene: Option<hygiene::HygieneReport>,
    pub fuzz: Vec<fuzz::FuzzReport>,
    pub safety: Option<safety::SafetyReport>,
    pub mutations: Option<MutationReport>,
    pub failures: Vec<String>,
}

/// Result of one property sweep.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SweepReport {
    pub function: String,
    pub iterations: u64,
    pub passed: bool,
}

impl Report {
    /// The V0 and V2 rungs passed. Mutation testing is reported separately and
    /// measured with [`MutationReport::kill_rate`].
    pub fn passed(&self) -> bool {
        self.compiled
            && self.calls_passed == self.calls_total
            && self.sweeps.iter().all(|sweep| sweep.passed)
            && self.hygiene.as_ref().is_none_or(|hygiene| hygiene.passed())
            && self.fuzz.iter().all(|fuzz| fuzz.passed)
            && self.safety.as_ref().is_none_or(|safety| safety.passed())
    }
}

/// A verification failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    Io { path: PathBuf, message: String },
    Json { path: PathBuf, message: String },
    Case(String),
    Process { tool: String, message: String },
    Unsupported(String),
    V0 { message: String },
}

impl Error {
    fn io(path: PathBuf, error: std::io::Error) -> Self {
        Self::Io {
            path,
            message: error.to_string(),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io { path, message } => write!(formatter, "{}: {message}", path.display()),
            Error::Json { path, message } => write!(formatter, "{}: {message}", path.display()),
            Error::Case(message) => formatter.write_str(message),
            Error::Process { tool, message } => write!(formatter, "{tool}: {message}"),
            Error::Unsupported(message) => formatter.write_str(message),
            Error::V0 { message } => write!(formatter, "V0 failed: {message}"),
        }
    }
}

impl std::error::Error for Error {}

/// Verify `program` against its C original.
pub fn verify(
    program: &Program,
    crate_dir: &Path,
    source: &Path,
    case: &Case,
    options: &Options,
) -> Result<Report, Error> {
    std::fs::create_dir_all(&options.work)
        .map_err(|error| Error::io(options.work.clone(), error))?;
    let case_name = source
        .parent()
        .and_then(|path| path.file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "case".to_string());

    let rlib = options.work.join("libgenerated.rlib");
    let v0 = compile_library(options, &crate_dir.join("src/lib.rs"), &rlib);
    if let Err(Error::V0 { message }) = v0 {
        return Ok(Report {
            case: case_name,
            compiled: false,
            calls_passed: 0,
            calls_total: case.calls.len(),
            sweeps: Vec::new(),
            hygiene: None,
            fuzz: Vec::new(),
            safety: None,
            mutations: None,
            failures: vec![message],
        });
    }
    v0?;

    let library_source = std::fs::read_to_string(crate_dir.join("src/lib.rs"))
        .map_err(|error| Error::io(crate_dir.join("src/lib.rs"), error))?;
    let hygiene = if options.hygiene {
        Some(hygiene::run(options, crate_dir, &library_source)?)
    } else {
        None
    };

    let c_driver = options.work.join("calls.c");
    let rust_driver = options.work.join("calls.rs");
    let drivers = driver::calls(program, case, source)?;
    write(&c_driver, &drivers.c)?;
    write(&rust_driver, &drivers.rust)?;

    let c_binary = options.work.join("calls-c");
    let rust_binary = options.work.join("calls-rust");
    compile_c(options, &c_driver, &c_binary)?;
    compile_rust_driver(options, &rust_driver, &rlib, &rust_binary)?;

    let c_run = run(&c_binary)?;
    let rust_run = run(&rust_binary)?;
    // Every input set the ladder executed is a place a mutant can be caught.
    // Mutation reuses the drivers and their C baselines rather than inventing
    // new inputs, so its measurement is exactly the evidence V2 and V3 supply.
    let mut mutation_inputs = vec![mutation::Input {
        rung: "v2-calls",
        driver: rust_driver.clone(),
        expected: c_run.clone(),
    }];
    let calls_total = case.calls.len();
    let calls_passed = if c_run == rust_run {
        calls_total
    } else {
        calls_total.saturating_sub(1)
    };
    let mut failures = Vec::new();
    if c_run != rust_run {
        failures.push(format!(
            "V2 differential failed\n--- C ---\n{}\n--- Rust ---\n{}",
            c_run, rust_run
        ));
    }

    let mut sweeps = Vec::new();
    for sweep in &case.sweeps {
        let sweep_drivers = driver::sweep(program, sweep, source)?;
        let stem = format!("sweep-{}", sanitize(&sweep.function));
        let c_path = options.work.join(format!("{stem}.c"));
        let rust_path = options.work.join(format!("{stem}.rs"));
        write(&c_path, &sweep_drivers.c)?;
        write(&rust_path, &sweep_drivers.rust)?;
        let c_bin = options.work.join(format!("{stem}-c"));
        let rust_bin = options.work.join(format!("{stem}-rust"));
        compile_c(options, &c_path, &c_bin)?;
        compile_rust_driver(options, &rust_path, &rlib, &rust_bin)?;
        let c_sweep = run(&c_bin)?;
        let passed = c_sweep == run(&rust_bin)?;
        mutation_inputs.push(mutation::Input {
            rung: "v2-sweeps",
            driver: rust_path.clone(),
            expected: c_sweep,
        });
        if !passed {
            failures.push(format!(
                "property sweep of {} diverged after {} iterations",
                sweep.function, sweep.iterations
            ));
        }
        sweeps.push(SweepReport {
            function: sweep.function.clone(),
            iterations: sweep.iterations,
            passed,
        });
    }

    let fuzz_run = if options.fuzz_executions > 0 {
        fuzz::run(options, program, case, source, options.fuzz_executions)?
    } else {
        None
    };
    let fuzz = fuzz_run
        .as_ref()
        .map(|fuzz| fuzz.reports.clone())
        .unwrap_or_default();
    if let Some(fuzz) = &fuzz_run {
        mutation_inputs.push(mutation::Input {
            rung: "v3-fuzz",
            driver: fuzz.driver.clone(),
            expected: fuzz.expected.clone(),
        });
    }
    if !fuzz.iter().all(|fuzz| fuzz.passed) {
        failures.push("V3 differential fuzzing diverged".to_string());
    }

    let safety = if options.sanitizers {
        Some(safety::run(options, &c_driver)?)
    } else {
        None
    };
    if let Some(safety) = &safety {
        if !safety.passed() {
            failures.push("V4 dynamic safety checks failed".to_string());
        }
    }

    let mutations = if options.mutation {
        // Mutation testing measures the strength of the differential inputs; a
        // survivor is a quality signal, not a wrong translation. The CLI decides
        // whether the configured kill rate is met (M1: 95%, M3: 98%).
        Some(mutation::run_mutation(
            options,
            &crate_dir.join("src/lib.rs"),
            &mutation_inputs,
        )?)
    } else {
        None
    };

    Ok(Report {
        case: case_name,
        compiled: true,
        calls_passed,
        calls_total,
        sweeps,
        hygiene,
        fuzz,
        safety,
        mutations,
        failures,
    })
}

pub(crate) fn write(path: &Path, text: &str) -> Result<(), Error> {
    std::fs::write(path, text).map_err(|error| Error::io(path.to_path_buf(), error))
}

pub(crate) fn compile_library(
    options: &Options,
    source: &Path,
    output: &Path,
) -> Result<(), Error> {
    let output = Command::new(&options.rustc)
        .args([
            "--edition=2021",
            "--crate-type=lib",
            "--crate-name",
            "generated",
        ])
        .arg("-C")
        .arg("overflow-checks=on")
        .arg(source)
        .arg("-o")
        .arg(output)
        .output()
        .map_err(|error| Error::io(options.rustc.clone(), error))?;
    if !output.status.success() {
        return Err(Error::V0 {
            message: String::from_utf8_lossy(&output.stderr).into_owned(),
        });
    }
    Ok(())
}

pub(crate) fn compile_c(options: &Options, source: &Path, output: &Path) -> Result<(), Error> {
    let output = Command::new(&options.cc)
        .arg("-std=c11")
        .arg("-O1")
        .arg("-o")
        .arg(output)
        .arg(source)
        .output()
        .map_err(|error| Error::io(options.cc.clone(), error))?;
    if !output.status.success() {
        return Err(Error::Process {
            tool: "clang".to_string(),
            message: String::from_utf8_lossy(&output.stderr).into_owned(),
        });
    }
    Ok(())
}

pub(crate) fn compile_rust_driver(
    options: &Options,
    source: &Path,
    rlib: &Path,
    output: &Path,
) -> Result<(), Error> {
    let output = Command::new(&options.rustc)
        .args(["--edition=2021", "-C", "overflow-checks=on", "--extern"])
        .arg(format!("generated={}", rlib.display()))
        .arg(source)
        .arg("-o")
        .arg(output)
        .output()
        .map_err(|error| Error::io(options.rustc.clone(), error))?;
    if !output.status.success() {
        return Err(Error::Process {
            tool: "rustc".to_string(),
            message: String::from_utf8_lossy(&output.stderr).into_owned(),
        });
    }
    Ok(())
}

pub(crate) fn run(binary: &Path) -> Result<String, Error> {
    run_with_timeout(binary, Duration::from_secs(60))
}

/// Run a binary, giving up after `timeout`. A mutant that loops forever is a
/// behavioral difference, not a reason to hang the pipeline.
pub(crate) fn run_with_timeout(binary: &Path, timeout: Duration) -> Result<String, Error> {
    let mut child = Command::new(binary)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| Error::io(binary.to_path_buf(), error))?;
    // Read both pipes while the child runs. Waiting first would deadlock as
    // soon as a driver wrote past the pipe buffer, which a fuzz run does.
    let mut stdout = child.stdout.take().expect("stdout is piped");
    let mut stderr = child.stderr.take().expect("stderr is piped");
    let stdout = std::thread::spawn(move || {
        let mut buffer = Vec::new();
        let _ = std::io::Read::read_to_end(&mut stdout, &mut buffer);
        buffer
    });
    let stderr = std::thread::spawn(move || {
        let mut buffer = Vec::new();
        let _ = std::io::Read::read_to_end(&mut stderr, &mut buffer);
        buffer
    });
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if start.elapsed() >= timeout => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = stdout.join();
                let _ = stderr.join();
                return Err(Error::Process {
                    tool: binary.display().to_string(),
                    message: format!("timed out after {}s", timeout.as_secs()),
                });
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(5)),
            Err(error) => return Err(Error::io(binary.to_path_buf(), error)),
        }
    };
    let stdout = stdout.join().unwrap_or_default();
    let stderr = stderr.join().unwrap_or_default();
    if !status.success() {
        return Err(Error::Process {
            tool: binary.display().to_string(),
            message: format!(
                "exited with {:?}: {}",
                status.code(),
                String::from_utf8_lossy(&stderr)
            ),
        });
    }
    Ok(String::from_utf8_lossy(&stdout).into_owned())
}

pub(crate) fn sanitize(name: &str) -> String {
    name.chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(mutations: Option<MutationReport>) -> Report {
        Report {
            case: "unit".to_string(),
            compiled: true,
            calls_passed: 1,
            calls_total: 1,
            sweeps: Vec::new(),
            hygiene: None,
            fuzz: Vec::new(),
            safety: None,
            mutations,
            failures: Vec::new(),
        }
    }

    #[test]
    fn mutation_kill_rate_counts_compile_failures_and_behavior() {
        let mutations = MutationReport {
            generated: 4,
            killed_by_behavior: 2,
            killed_by_compile: 1,
            by_rung: BTreeMap::from([
                ("v2-calls".to_string(), 1),
                ("v2-sweeps".to_string(), 0),
                ("v3-fuzz".to_string(), 1),
            ]),
            killed_by_check: 0,
            survived: vec!["m003".to_string()],
        };
        assert_eq!(mutations.killed(), 3);
        assert_eq!(mutations.kill_rate(), 75.0);
    }

    #[test]
    fn rung_counts_are_reported_in_ladder_order() {
        let mutations = MutationReport {
            generated: 3,
            killed_by_behavior: 3,
            killed_by_compile: 0,
            by_rung: BTreeMap::from([("v3-fuzz".to_string(), 1), ("v2-calls".to_string(), 2)]),
            killed_by_check: 1,
            survived: Vec::new(),
        };
        assert_eq!(mutations.rungs(), vec![("v2-calls", 2), ("v3-fuzz", 1)]);
        let attributed: usize = mutations.rungs().iter().map(|(_, count)| count).sum();
        assert_eq!(attributed, mutations.killed_by_behavior);
    }

    #[test]
    fn surviving_mutants_do_not_fail_the_verification_rungs() {
        let mutations = MutationReport {
            generated: 2,
            killed_by_behavior: 1,
            killed_by_compile: 0,
            by_rung: BTreeMap::from([("v2-calls".to_string(), 1)]),
            killed_by_check: 0,
            survived: vec!["m001".to_string()],
        };
        assert!(report(Some(mutations)).passed());
    }

    #[test]
    fn a_failed_call_fails_verification() {
        let mut report = report(None);
        report.calls_passed = 0;
        assert!(!report.passed());
    }
}
