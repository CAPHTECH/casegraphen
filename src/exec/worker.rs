use super::binding::{WorkerBinding, WorkerKind};
use higher_graphen_core::Id;
use std::{
    env, fmt,
    io::{self, Read},
    path::{Path, PathBuf},
    process::{Child, Command, ExitCode, Stdio},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const OUTPUT_LIMIT_BYTES: usize = 4 * 1024 * 1024;
const POLL_INTERVAL: Duration = Duration::from_millis(10);

pub type CliExitCode = ExitCode;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkerContext {
    pub run_directory: PathBuf,
    pub input_report_path: PathBuf,
    pub case_space_id: Id,
    pub plan_id: Id,
    pub step_id: Id,
    pub work_cell_id: Id,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkerInvocation {
    pub exit_status: Option<i32>,
    pub timed_out: bool,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
    pub started_at: String,
    pub finished_at: String,
    pub stdout_sha256: String,
    pub stderr_sha256: String,
}

#[derive(Debug)]
pub struct WorkerError {
    message: String,
}

impl WorkerError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for WorkerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for WorkerError {}

pub trait Worker {
    fn execute(
        &self,
        binding: &WorkerBinding,
        ctx: &WorkerContext,
    ) -> Result<WorkerInvocation, WorkerError>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ShellWorker;

impl Worker for ShellWorker {
    fn execute(
        &self,
        binding: &WorkerBinding,
        ctx: &WorkerContext,
    ) -> Result<WorkerInvocation, WorkerError> {
        let working_directory = Path::new(&binding.working_directory);
        if !working_directory.exists() {
            return Err(WorkerError::new(format!(
                "worker binding {} working_directory {} does not exist",
                binding.binding_id,
                working_directory.display()
            )));
        }

        let started_at = timestamp();
        let started = Instant::now();
        let mut command = Command::new(&binding.command);
        command
            .args(&binding.args)
            .current_dir(working_directory)
            .env_clear()
            .env("CASEGRAPHEN_INPUT_REPORT", &ctx.input_report_path)
            .env("CASEGRAPHEN_RUN_DIR", &ctx.run_directory)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for name in &binding.env_allowlist {
            if let Some(value) = env::var_os(name) {
                command.env(name, value);
            }
        }

        let mut child = command.spawn().map_err(|error| {
            WorkerError::new(format!(
                "failed to spawn shell worker {} command {:?}: {error}",
                binding.binding_id, binding.command
            ))
        })?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| WorkerError::new("shell worker stdout pipe was not captured"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| WorkerError::new("shell worker stderr pipe was not captured"))?;
        let stdout_reader = thread::spawn(move || read_capped(stdout));
        let stderr_reader = thread::spawn(move || read_capped(stderr));

        let timeout = Duration::from_millis(binding.timeout_ms);
        let (status, timed_out) = wait_with_timeout(&mut child, started, timeout)?;
        let stdout = join_reader(stdout_reader, "stdout")?;
        let stderr = join_reader(stderr_reader, "stderr")?;
        let finished_at = timestamp();

        Ok(WorkerInvocation {
            exit_status: if timed_out { None } else { status.code() },
            timed_out,
            stdout_sha256: crate::native_hash::sha256_hex(&stdout.bytes),
            stderr_sha256: crate::native_hash::sha256_hex(&stderr.bytes),
            stdout: stdout.bytes,
            stderr: stderr.bytes,
            stdout_truncated: stdout.truncated,
            stderr_truncated: stderr.truncated,
            started_at,
            finished_at,
        })
    }
}

pub fn execute_worker(
    binding: &WorkerBinding,
    ctx: &WorkerContext,
) -> Result<WorkerInvocation, WorkerError> {
    match binding.worker_kind {
        WorkerKind::Shell => ShellWorker.execute(binding, ctx),
    }
}

fn wait_with_timeout(
    child: &mut Child,
    started: Instant,
    timeout: Duration,
) -> Result<(std::process::ExitStatus, bool), WorkerError> {
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok((status, false)),
            Ok(None) if started.elapsed() >= timeout => {
                child.kill().map_err(|error| {
                    WorkerError::new(format!("failed to kill timed-out shell worker: {error}"))
                })?;
                let status = child.wait().map_err(|error| {
                    WorkerError::new(format!("failed to reap timed-out shell worker: {error}"))
                })?;
                return Ok((status, true));
            }
            Ok(None) => thread::sleep(POLL_INTERVAL.min(timeout)),
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(WorkerError::new(format!(
                    "failed while polling shell worker: {error}"
                )));
            }
        }
    }
}

struct CapturedOutput {
    bytes: Vec<u8>,
    truncated: bool,
}

fn read_capped(mut reader: impl Read) -> io::Result<CapturedOutput> {
    let mut bytes = Vec::new();
    let mut truncated = false;
    let mut buffer = [0_u8; 8192];
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        let remaining = OUTPUT_LIMIT_BYTES.saturating_sub(bytes.len());
        let retained = remaining.min(count);
        bytes.extend_from_slice(&buffer[..retained]);
        if retained < count {
            truncated = true;
        }
    }
    Ok(CapturedOutput { bytes, truncated })
}

fn join_reader(
    handle: thread::JoinHandle<io::Result<CapturedOutput>>,
    stream: &str,
) -> Result<CapturedOutput, WorkerError> {
    handle
        .join()
        .map_err(|_| WorkerError::new(format!("shell worker {stream} reader panicked")))?
        .map_err(|error| {
            WorkerError::new(format!("failed to capture shell worker {stream}: {error}"))
        })
}

fn timestamp() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    format!("unix:{seconds}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::binding::{
        validate_worker_binding, WORKER_BINDING_SCHEMA, WORKER_BINDING_SCHEMA_VERSION,
    };
    use serde_json::Map;
    use std::{
        fs,
        sync::atomic::{AtomicU64, Ordering},
    };

    static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn shell_worker_passes_only_allowlisted_parent_environment() {
        let directory = test_directory("env");
        let allowed = format!(
            "CASEGRAPHEN_WORKER_ALLOWED_{}",
            TEST_COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let blocked = format!(
            "CASEGRAPHEN_WORKER_BLOCKED_{}",
            TEST_COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        env::set_var(&allowed, "visible");
        env::set_var(&blocked, "hidden");
        let script =
            format!("printf '%s|%s' \"${{{allowed}-missing}}\" \"${{{blocked}-missing}}\"");
        let mut binding = binding(&directory, &script, 2000);
        binding.env_allowlist = vec![allowed.clone()];

        let invocation = ShellWorker
            .execute(&binding, &context(&directory))
            .expect("execute shell worker");

        env::remove_var(allowed);
        env::remove_var(blocked);
        assert_eq!(invocation.exit_status, Some(0));
        assert_eq!(invocation.stdout, b"visible|missing");
        fs::remove_dir_all(directory).expect("remove worker test directory");
    }

    #[test]
    fn shell_worker_kills_a_timed_out_process() {
        let directory = test_directory("timeout");
        let binding = binding(&directory, "exec sleep 5", 200);
        let started = Instant::now();

        let invocation = ShellWorker
            .execute(&binding, &context(&directory))
            .expect("execute timed worker");

        assert!(invocation.timed_out);
        assert_eq!(invocation.exit_status, None);
        assert!(started.elapsed() < Duration::from_secs(3));
        fs::remove_dir_all(directory).expect("remove worker test directory");
    }

    #[test]
    fn shell_worker_captures_stdout_and_hashes_it() {
        let directory = test_directory("stdout");
        let binding = binding(&directory, "printf 'captured-output'", 2000);

        let invocation = execute_worker(&binding, &context(&directory)).expect("execute worker");

        assert_eq!(invocation.stdout, b"captured-output");
        assert_eq!(
            invocation.stdout_sha256,
            crate::native_hash::sha256_hex(b"captured-output")
        );
        assert!(!invocation.stdout_truncated);
        fs::remove_dir_all(directory).expect("remove worker test directory");
    }

    fn binding(directory: &Path, script: &str, timeout_ms: u64) -> WorkerBinding {
        let binding = WorkerBinding {
            schema: WORKER_BINDING_SCHEMA.to_owned(),
            schema_version: WORKER_BINDING_SCHEMA_VERSION,
            binding_id: Id::new("worker_binding:shell-worker-test").expect("binding id"),
            worker_kind: WorkerKind::Shell,
            command: "/bin/sh".to_owned(),
            args: vec!["-c".to_owned(), script.to_owned()],
            working_directory: directory.display().to_string(),
            env_allowlist: Vec::new(),
            timeout_ms,
            capability_ids: vec![Id::new("capability:shell-worker-test").expect("capability id")],
            metadata: Map::new(),
        };
        validate_worker_binding(&binding).expect("valid binding");
        binding
    }

    fn context(directory: &Path) -> WorkerContext {
        let input_report_path = directory.join("input.report.json");
        fs::write(&input_report_path, "{}\n").expect("write input report");
        WorkerContext {
            run_directory: directory.to_path_buf(),
            input_report_path,
            case_space_id: Id::new("case_space:worker-test").expect("case space id"),
            plan_id: Id::new("plan:worker-test").expect("plan id"),
            step_id: Id::new("step:worker-test").expect("step id"),
            work_cell_id: Id::new("work:worker-test").expect("work cell id"),
        }
    }

    fn test_directory(label: &str) -> PathBuf {
        let counter = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = env::temp_dir().join(format!(
            "casegraphen-worker-{label}-{}-{counter}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("create worker test directory");
        path
    }
}
