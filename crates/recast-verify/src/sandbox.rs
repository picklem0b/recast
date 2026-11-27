//! A restricted process runner for code the verifier executes.
//!
//! The sandbox gives every run a scratch working directory, an empty
//! environment, CPU, address-space and file-size limits, and a wall-clock
//! deadline. Network and mount isolation need a container runtime that is not
//! assumed here; [`Sandbox::limitations`] names what the runner does not do so
//! the report can state it instead of implying a stronger guarantee.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::Error;

/// Limits and scratch space for one sandboxed run.
#[derive(Debug, Clone)]
pub struct Sandbox {
    pub scratch: PathBuf,
    pub timeout: Duration,
    pub cpu_seconds: u64,
    pub memory_kb: u64,
    pub file_blocks: u64,
}

/// The captured result of a sandboxed process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxOutput {
    pub status: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
}

impl SandboxOutput {
    /// The process exited with status zero and did not time out.
    pub fn success(&self) -> bool {
        !self.timed_out && self.status == Some(0)
    }
}

impl Sandbox {
    /// Build a runner rooted at `scratch`.
    pub fn new(scratch: impl Into<PathBuf>) -> Self {
        Self {
            scratch: scratch.into(),
            timeout: Duration::from_secs(60),
            cpu_seconds: 30,
            memory_kb: 2 * 1024 * 1024,
            file_blocks: 64 * 1024,
        }
    }

    /// What this runner does not isolate, in the report's words.
    pub fn limitations(&self) -> &'static str {
        "runs use a scratch directory, a cleared environment (toolchain checks keep the variables Cargo needs), CPU, memory and file-size limits, and a timeout; network and mount isolation need a container runtime that this build does not assume"
    }

    /// Run `binary` with no arguments.
    pub fn run(&self, binary: &Path) -> Result<SandboxOutput, Error> {
        self.run_with_args(binary, &[])
    }

    /// Run `binary` with `args` in a cleared environment.
    pub fn run_with_args(&self, binary: &Path, args: &[String]) -> Result<SandboxOutput, Error> {
        self.run_with_env(binary, args, &[])
    }

    /// Run `binary` with `args` and the `env` variables, and nothing else.
    /// A Cargo check needs `HOME`, `PATH` and the toolchain variables; the
    /// compiled programs this runner executes need none.
    pub fn run_with_env(
        &self,
        binary: &Path,
        args: &[String],
        env: &[(String, String)],
    ) -> Result<SandboxOutput, Error> {
        // The process starts in the scratch directory, so a relative binary or
        // scratch path would resolve there instead of against the caller's
        // working directory. Pin both to absolute paths first.
        let binary = std::path::absolute(binary).map_err(|error| Error::Io {
            path: binary.to_path_buf(),
            message: error.to_string(),
        })?;
        let scratch = std::path::absolute(&self.scratch).map_err(|error| Error::Io {
            path: self.scratch.clone(),
            message: error.to_string(),
        })?;
        std::fs::create_dir_all(&scratch).map_err(|error| Error::Io {
            path: scratch.clone(),
            message: error.to_string(),
        })?;
        // `ulimit -v` is advisory on some hosts and `-f` counts blocks, so the
        // script ignores an unsupported limit rather than refusing to run.
        let script = format!(
            "ulimit -t {} 2>/dev/null; ulimit -v {} 2>/dev/null; ulimit -f {} 2>/dev/null; exec \"$0\" \"$@\"",
            self.cpu_seconds, self.memory_kb, self.file_blocks
        );
        let mut command = Command::new("bash");
        command
            .arg("-c")
            .arg(&script)
            .arg(&binary)
            .args(args)
            .current_dir(&scratch)
            .env_clear()
            .envs(env.iter().map(|(key, value)| (key, value)))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(Stdio::null());
        let mut child = command.spawn().map_err(|error| Error::Io {
            path: binary.clone(),
            message: error.to_string(),
        })?;
        let start = Instant::now();
        let timed_out = loop {
            match child.try_wait() {
                Ok(Some(_)) => break false,
                Ok(None) if start.elapsed() >= self.timeout => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break true;
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(5)),
                Err(error) => {
                    return Err(Error::Io {
                        path: binary.clone(),
                        message: error.to_string(),
                    })
                }
            }
        };
        let mut stdout = String::new();
        let mut stderr = String::new();
        if let Some(mut pipe) = child.stdout.take() {
            let _ = pipe.read_to_string(&mut stdout);
        }
        if let Some(mut pipe) = child.stderr.take() {
            let _ = pipe.read_to_string(&mut stderr);
        }
        let status = child.wait().ok().and_then(|status| status.code());
        Ok(SandboxOutput {
            status,
            stdout,
            stderr,
            timed_out,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_clean_process_reports_success_and_output() {
        let dir = std::env::temp_dir().join(format!("recast-sandbox-{}", std::process::id()));
        let sandbox = Sandbox::new(dir);
        let output = sandbox
            .run_with_args(
                Path::new("/bin/sh"),
                &["-c".to_string(), "printf ok".to_string()],
            )
            .expect("run");
        assert!(output.success(), "{output:?}");
        assert_eq!(output.stdout, "ok");
    }

    #[test]
    fn only_the_passed_environment_variables_survive() {
        let dir = std::env::temp_dir().join(format!("recast-sandbox-env-{}", std::process::id()));
        let sandbox = Sandbox::new(dir);
        let output = sandbox
            .run_with_env(
                Path::new("/bin/sh"),
                &[
                    "-c".to_string(),
                    "printf %s \"$RECAST_SANDBOX_PROBE\"".to_string(),
                ],
                &[("RECAST_SANDBOX_PROBE".to_string(), "inner".to_string())],
            )
            .expect("run");
        assert_eq!(output.stdout, "inner");
    }

    #[test]
    fn a_failing_process_keeps_its_status_and_stderr() {
        let dir = std::env::temp_dir().join(format!("recast-sandbox-fail-{}", std::process::id()));
        let sandbox = Sandbox::new(dir);
        let output = sandbox
            .run_with_args(
                Path::new("/bin/sh"),
                &["-c".to_string(), "echo bad >&2; exit 7".to_string()],
            )
            .expect("run");
        assert_eq!(output.status, Some(7));
        assert!(output.stderr.contains("bad"), "{output:?}");
    }
}
