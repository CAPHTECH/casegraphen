use super::binding::{validate_worker_binding, WorkerBinding, WorkerKind};
use crate::native_hash::Sha256;
use higher_graphen_core::Id;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::{
    env, fmt, fs,
    io::{self, Read},
    path::{Path, PathBuf},
    process::{Child, Command, ExitCode, ExitStatus, Stdio},
    sync::{Arc, Mutex},
    thread::{self, JoinHandle},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const OUTPUT_LIMIT_BYTES: usize = 4 * 1024 * 1024;
const POLL_INTERVAL: Duration = Duration::from_millis(10);
const PROCESS_REAP_GRACE: Duration = Duration::from_millis(50);
const READER_GRACE: Duration = Duration::from_secs(2);

#[cfg(unix)]
const SETSID_CANDIDATES: &[&str] = &[
    "/usr/bin/setsid",
    "/bin/setsid",
    "/usr/local/bin/setsid",
    "/opt/homebrew/bin/setsid",
];
#[cfg(unix)]
const KILL_CANDIDATES: &[&str] = &["/bin/kill", "/usr/bin/kill"];

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
    pub descendants_may_survive: bool,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub stdout_byte_len: u64,
    pub stderr_byte_len: u64,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
    pub stdout_incomplete: bool,
    pub stderr_incomplete: bool,
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
        validate_worker_binding(binding).map_err(|error| {
            WorkerError::new(format!(
                "invalid worker binding {}: {error}",
                binding.binding_id
            ))
        })?;
        let working_directory = fs::canonicalize(&binding.working_directory).map_err(|error| {
            WorkerError::new(format!(
                "worker binding {} working_directory {} could not be canonicalized: {error}",
                binding.binding_id, binding.working_directory
            ))
        })?;
        if !working_directory.is_dir() {
            return Err(WorkerError::new(format!(
                "worker binding {} canonical working_directory {} is not a directory",
                binding.binding_id,
                working_directory.display()
            )));
        }
        let command_path = fs::canonicalize(&binding.command).map_err(|error| {
            WorkerError::new(format!(
                "worker binding {} command {} could not be canonicalized: {error}",
                binding.binding_id, binding.command
            ))
        })?;
        if !command_path.is_file() {
            return Err(WorkerError::new(format!(
                "worker binding {} canonical command {} is not a file",
                binding.binding_id,
                command_path.display()
            )));
        }
        require_executable(binding, &command_path)?;

        let started_at = timestamp();
        let started = Instant::now();
        let (mut command, process_group_kill) = contained_command(&command_path);
        command
            .args(&binding.args)
            .current_dir(&working_directory)
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
        let stdout_reader = spawn_reader(stdout);
        let stderr_reader = spawn_reader(stderr);

        let timeout = Duration::from_millis(binding.timeout_ms);
        let mut outcome =
            wait_with_timeout(&mut child, started, timeout, process_group_kill.as_deref())?;
        let (stdout, stderr) = finish_readers(stdout_reader, stderr_reader)?;
        if stdout.incomplete || stderr.incomplete {
            outcome.process_group = finalize_process_group(
                process_group_kill.as_deref(),
                child.id(),
                WorkerExitKind::IncompleteOutput,
                outcome.process_group.signal_outcome,
            );
        }
        let finished_at = timestamp();

        Ok(WorkerInvocation {
            exit_status: if outcome.timed_out {
                None
            } else {
                outcome.status.and_then(|status| status.code())
            },
            timed_out: outcome.timed_out,
            descendants_may_survive: outcome.process_group.decision.descendants_may_survive,
            stdout_sha256: stdout.content_hash,
            stderr_sha256: stderr.content_hash,
            stdout_byte_len: stdout.byte_len,
            stderr_byte_len: stderr.byte_len,
            stdout: stdout.bytes,
            stderr: stderr.bytes,
            stdout_truncated: stdout.truncated,
            stderr_truncated: stderr.truncated,
            stdout_incomplete: stdout.incomplete,
            stderr_incomplete: stderr.incomplete,
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
    process_group_kill: Option<&Path>,
) -> Result<WaitOutcome, WorkerError> {
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let process_group = finalize_process_group(
                    process_group_kill,
                    child.id(),
                    WorkerExitKind::Clean,
                    None,
                );
                return Ok(WaitOutcome {
                    status: Some(status),
                    timed_out: false,
                    process_group,
                });
            }
            Ok(None) if started.elapsed() >= timeout => {
                let process_group = finalize_process_group(
                    process_group_kill,
                    child.id(),
                    WorkerExitKind::Timeout,
                    None,
                );
                if process_group.decision.descendants_may_survive {
                    child.kill().map_err(|error| {
                        WorkerError::new(format!("failed to kill timed-out shell worker: {error}"))
                    })?;
                }
                let status = reap_bounded(child, PROCESS_REAP_GRACE)?;
                return Ok(WaitOutcome {
                    status,
                    timed_out: true,
                    process_group,
                });
            }
            Ok(None) => thread::sleep(POLL_INTERVAL.min(timeout)),
            Err(error) => {
                let process_group = finalize_process_group(
                    process_group_kill,
                    child.id(),
                    WorkerExitKind::PollFailure,
                    None,
                );
                if process_group.decision.descendants_may_survive {
                    let _ = child.kill();
                }
                return Err(WorkerError::new(format!(
                    "failed while polling shell worker: {error}"
                )));
            }
        }
    }
}

struct WaitOutcome {
    status: Option<ExitStatus>,
    timed_out: bool,
    process_group: ProcessGroupFinalization,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WorkerExitKind {
    Clean,
    Timeout,
    IncompleteOutput,
    PollFailure,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ProcessGroupExitDecision {
    signal_process_group: bool,
    descendants_may_survive: bool,
}

struct ProcessGroupFinalization {
    decision: ProcessGroupExitDecision,
    signal_outcome: Option<SignalOutcome>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SignalOutcome {
    Terminated,
    GroupAlreadyEmpty,
    Failed,
}

fn process_group_exit_decision(
    utilities_available: bool,
    exit_kind: WorkerExitKind,
    signal_outcome: Option<SignalOutcome>,
) -> ProcessGroupExitDecision {
    // Containment and its durable claim do not depend on how the worker ended.
    let group_contained = matches!(
        signal_outcome,
        Some(SignalOutcome::Terminated | SignalOutcome::GroupAlreadyEmpty)
    );
    match exit_kind {
        WorkerExitKind::Clean
        | WorkerExitKind::Timeout
        | WorkerExitKind::IncompleteOutput
        | WorkerExitKind::PollFailure => ProcessGroupExitDecision {
            signal_process_group: utilities_available && !group_contained,
            descendants_may_survive: !(utilities_available && group_contained),
        },
    }
}

fn finalize_process_group(
    process_group_kill: Option<&Path>,
    child_id: u32,
    exit_kind: WorkerExitKind,
    signal_outcome: Option<SignalOutcome>,
) -> ProcessGroupFinalization {
    let utilities_available = process_group_kill.is_some();
    let decision = process_group_exit_decision(utilities_available, exit_kind, signal_outcome);
    let signal_outcome = if decision.signal_process_group {
        process_group_kill.map(|kill| kill_process_group(kill, child_id))
    } else {
        signal_outcome
    };
    ProcessGroupFinalization {
        decision: process_group_exit_decision(utilities_available, exit_kind, signal_outcome),
        signal_outcome,
    }
}

fn reap_bounded(child: &mut Child, grace: Duration) -> Result<Option<ExitStatus>, WorkerError> {
    let deadline = Instant::now() + grace;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(Some(status)),
            Ok(None) if Instant::now() >= deadline => return Ok(None),
            Ok(None) => thread::sleep(POLL_INTERVAL.min(grace)),
            Err(error) => {
                return Err(WorkerError::new(format!(
                    "failed to reap timed-out shell worker: {error}"
                )))
            }
        }
    }
}

/// Refuses a command that carries no execute bit at all.
///
/// Without this the classification of an unrunnable command depends on the host.
/// When `setsid` is available the worker is launched as `setsid <command>`, so an
/// exec failure surfaces as a nonzero exit status of `setsid` — reported as "the
/// worker ran and failed", which attaches worker evidence. Without `setsid` the
/// command is spawned directly and the same binding produces a dispatch error.
/// Checking before the spawn makes "could not be executed" mean the same thing
/// on every host.
#[cfg(unix)]
fn require_executable(binding: &WorkerBinding, command_path: &Path) -> Result<(), WorkerError> {
    use std::os::unix::fs::PermissionsExt;

    let mode = fs::metadata(command_path)
        .map_err(|error| {
            WorkerError::new(format!(
                "worker binding {} canonical command {} could not be inspected: {error}",
                binding.binding_id,
                command_path.display()
            ))
        })?
        .permissions()
        .mode();
    if mode & 0o111 == 0 {
        return Err(WorkerError::new(format!(
            "worker binding {} canonical command {} is not executable (mode {:o})",
            binding.binding_id,
            command_path.display(),
            mode & 0o7777
        )));
    }
    Ok(())
}

#[cfg(not(unix))]
fn require_executable(_binding: &WorkerBinding, _command_path: &Path) -> Result<(), WorkerError> {
    Ok(())
}

fn contained_command(command_path: &Path) -> (Command, Option<PathBuf>) {
    if let Some((setsid, kill)) = process_group_utilities() {
        let mut command = Command::new(setsid);
        command.arg(command_path);
        (command, Some(kill))
    } else {
        (Command::new(command_path), None)
    }
}

#[cfg(unix)]
fn process_group_utilities() -> Option<(PathBuf, PathBuf)> {
    Some((
        find_executable(SETSID_CANDIDATES)?,
        find_executable(KILL_CANDIDATES)?,
    ))
}

#[cfg(not(unix))]
fn process_group_utilities() -> Option<(PathBuf, PathBuf)> {
    None
}

#[cfg(unix)]
fn find_executable(candidates: &[&str]) -> Option<PathBuf> {
    candidates.iter().map(PathBuf::from).find(|path| {
        fs::metadata(path)
            .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
    })
}

fn kill_process_group(kill_path: &Path, child_id: u32) -> SignalOutcome {
    let kill = signal_process_group(kill_path, "-KILL", child_id);
    let probe = if kill == Some(true) {
        None
    } else {
        signal_process_group(kill_path, "-0", child_id)
    };
    signal_outcome(kill, probe)
}

/// `GroupAlreadyEmpty` is only concluded from a probe that actually ran and
/// reported no such process. A probe the host could not spawn measured
/// nothing, so it cannot claim the group is empty.
fn signal_outcome(kill: Option<bool>, probe: Option<bool>) -> SignalOutcome {
    match (kill, probe) {
        (Some(true), _) => SignalOutcome::Terminated,
        (_, Some(false)) => SignalOutcome::GroupAlreadyEmpty,
        _ => SignalOutcome::Failed,
    }
}

/// `None` when the utility could not be run at all, which is not the same
/// answer as its running and reporting failure.
fn signal_process_group(kill_path: &Path, signal: &str, child_id: u32) -> Option<bool> {
    Command::new(kill_path)
        .args([signal, "--"])
        .arg(format!("-{child_id}"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .ok()
        .map(|status| status.success())
}

#[derive(Clone)]
struct CaptureProgress {
    bytes: Vec<u8>,
    byte_len: u64,
    truncated: bool,
    hasher: Sha256,
}

impl CaptureProgress {
    fn new() -> Self {
        Self {
            bytes: Vec::new(),
            byte_len: 0,
            truncated: false,
            hasher: Sha256::new(),
        }
    }

    fn record(&mut self, bytes: &[u8]) {
        self.hasher.update(bytes);
        self.byte_len = self.byte_len.saturating_add(bytes.len() as u64);
        let remaining = OUTPUT_LIMIT_BYTES.saturating_sub(self.bytes.len());
        let retained = remaining.min(bytes.len());
        self.bytes.extend_from_slice(&bytes[..retained]);
        if retained < bytes.len() {
            self.truncated = true;
        }
    }
}

struct CapturedOutput {
    bytes: Vec<u8>,
    byte_len: u64,
    truncated: bool,
    incomplete: bool,
    content_hash: String,
}

#[cfg(test)]
fn read_capped(mut reader: impl Read) -> io::Result<CapturedOutput> {
    let capture = Arc::new(Mutex::new(CaptureProgress::new()));
    read_into_capture(&mut reader, &capture)?;
    Ok(capture_snapshot(&capture, false))
}

fn read_into_capture(
    mut reader: impl Read,
    capture: &Arc<Mutex<CaptureProgress>>,
) -> io::Result<()> {
    let mut buffer = [0_u8; 8192];
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        capture
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .record(&buffer[..count]);
    }
    Ok(())
}

struct OutputReader {
    handle: JoinHandle<io::Result<()>>,
    capture: Arc<Mutex<CaptureProgress>>,
}

fn spawn_reader(reader: impl Read + Send + 'static) -> OutputReader {
    let capture = Arc::new(Mutex::new(CaptureProgress::new()));
    let reader_capture = Arc::clone(&capture);
    let handle = thread::spawn(move || read_into_capture(reader, &reader_capture));
    OutputReader { handle, capture }
}

fn finish_readers(
    stdout: OutputReader,
    stderr: OutputReader,
) -> Result<(CapturedOutput, CapturedOutput), WorkerError> {
    let deadline = Instant::now() + READER_GRACE;
    while (!stdout.handle.is_finished() || !stderr.handle.is_finished())
        && Instant::now() < deadline
    {
        thread::sleep(POLL_INTERVAL);
    }
    Ok((
        finish_reader(stdout, "stdout")?,
        finish_reader(stderr, "stderr")?,
    ))
}

fn finish_reader(reader: OutputReader, stream: &str) -> Result<CapturedOutput, WorkerError> {
    if !reader.handle.is_finished() {
        return Ok(capture_snapshot(&reader.capture, true));
    }
    reader
        .handle
        .join()
        .map_err(|_| WorkerError::new(format!("shell worker {stream} reader panicked")))?
        .map_err(|error| {
            WorkerError::new(format!("failed to capture shell worker {stream}: {error}"))
        })?;
    Ok(capture_snapshot(&reader.capture, false))
}

fn capture_snapshot(capture: &Arc<Mutex<CaptureProgress>>, incomplete: bool) -> CapturedOutput {
    let progress = capture
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    CapturedOutput {
        bytes: progress.bytes.clone(),
        byte_len: progress.byte_len,
        truncated: progress.truncated || incomplete,
        incomplete,
        content_hash: progress.hasher.clone().finalize_hex(),
    }
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
            "WORKER_ALLOWED_{}",
            TEST_COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let blocked = format!(
            "WORKER_BLOCKED_{}",
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
        let binding = binding(&directory, "sleep 5 & wait", 200);
        let started = Instant::now();

        let invocation = ShellWorker
            .execute(&binding, &context(&directory))
            .expect("execute timed worker");

        assert!(invocation.timed_out);
        assert_eq!(invocation.exit_status, None);
        assert!(started.elapsed() < Duration::from_millis(2500));
        fs::remove_dir_all(directory).expect("remove worker test directory");
    }

    #[test]
    fn process_group_report_matches_conclusive_signal_outcomes() {
        arbtest::arbtest(
            |u: &mut arbtest::arbitrary::Unstructured<'_>| -> arbtest::arbitrary::Result<()> {
                let utilities_available: bool = u.arbitrary()?;
                let exit_kind = *u.choose(&[
                    WorkerExitKind::Clean,
                    WorkerExitKind::Timeout,
                    WorkerExitKind::IncompleteOutput,
                ])?;
                let signal_outcome = *u.choose(&[
                    SignalOutcome::Terminated,
                    SignalOutcome::GroupAlreadyEmpty,
                    SignalOutcome::Failed,
                ])?;

                let decision = process_group_exit_decision(
                    utilities_available,
                    exit_kind,
                    Some(signal_outcome),
                );
                let conclusive = matches!(
                    signal_outcome,
                    SignalOutcome::Terminated | SignalOutcome::GroupAlreadyEmpty
                );

                assert_eq!(
                    !decision.descendants_may_survive,
                    utilities_available && conclusive
                );
                Ok(())
            },
        );
    }

    #[test]
    fn a_signal_that_never_ran_never_claims_an_empty_group() {
        arbtest::arbtest(
            |u: &mut arbtest::arbitrary::Unstructured<'_>| -> arbtest::arbitrary::Result<()> {
                let outcomes = [Some(true), Some(false), None];
                let kill = *u.choose(&outcomes)?;
                let probe = *u.choose(&outcomes)?;

                match signal_outcome(kill, probe) {
                    SignalOutcome::Terminated => assert_eq!(kill, Some(true)),
                    SignalOutcome::GroupAlreadyEmpty => {
                        assert_ne!(kill, Some(true));
                        assert_eq!(probe, Some(false));
                    }
                    SignalOutcome::Failed => assert!(kill != Some(true) && probe != Some(false)),
                }
                Ok(())
            },
        );
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

    #[test]
    fn shell_worker_rejects_an_absolute_command_that_does_not_exist() {
        let directory = test_directory("missing-command");
        let mut binding = binding(&directory, "exit 0", 2000);
        binding.command = directory.join("missing-command").display().to_string();

        let error = execute_worker(&binding, &context(&directory))
            .expect_err("missing canonical command must fail");

        assert!(error.to_string().contains("command"));
        assert!(error.to_string().contains("could not be canonicalized"));
        fs::remove_dir_all(directory).expect("remove worker test directory");
    }

    #[test]
    fn shell_worker_rejects_a_working_directory_that_is_not_a_directory() {
        let directory = test_directory("working-directory-file");
        let not_a_directory = directory.join("not-a-directory");
        fs::write(&not_a_directory, "file").expect("write non-directory fixture");
        let mut binding = binding(&directory, "exit 0", 2000);
        binding.working_directory = not_a_directory.display().to_string();

        let error = execute_worker(&binding, &context(&directory))
            .expect_err("canonical working directory file must fail");

        assert!(error.to_string().contains("is not a directory"));
        fs::remove_dir_all(directory).expect("remove worker test directory");
    }

    #[test]
    fn capped_output_hash_covers_the_full_stream() {
        let mut full_output = vec![b'a'; OUTPUT_LIMIT_BYTES];
        full_output.extend_from_slice(b"different-suffix");

        let captured = read_capped(full_output.as_slice()).expect("capture output");

        assert!(captured.truncated);
        assert_eq!(captured.bytes.len(), OUTPUT_LIMIT_BYTES);
        assert_eq!(captured.byte_len, full_output.len() as u64);
        assert_eq!(
            captured.content_hash,
            crate::native_hash::sha256_hex(&full_output),
            "the evidence hash must cover bytes beyond the retained prefix"
        );
    }

    /// The end-to-end dispatch test covers this too, but only on the spawn path
    /// this host happens to take. Asserting the check directly proves the
    /// classification does not depend on whether `setsid` exists here.
    #[cfg(unix)]
    #[test]
    fn a_command_without_any_execute_bit_is_refused_before_spawning() {
        use std::os::unix::fs::PermissionsExt;

        let directory = test_directory("not-executable");
        let command = directory.join("worker");
        fs::write(&command, b"#!/bin/sh\nexit 0\n").expect("write worker");
        let mut permissions = fs::metadata(&command).expect("metadata").permissions();
        permissions.set_mode(0o644);
        fs::set_permissions(&command, permissions).expect("drop execute bits");

        let refused = require_executable(&binding(&directory, "exit 0", 1000), &command)
            .expect_err("a non-executable command must be refused");
        assert!(
            refused.to_string().contains("is not executable"),
            "unexpected message: {refused}"
        );

        let mut permissions = fs::metadata(&command).expect("metadata").permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&command, permissions).expect("restore execute bit");
        require_executable(&binding(&directory, "exit 0", 1000), &command)
            .expect("an executable command must be accepted");

        fs::remove_dir_all(directory).expect("remove worker test directory");
    }

    fn binding(directory: &Path, script: &str, timeout_ms: u64) -> WorkerBinding {
        let mut binding = WorkerBinding {
            schema: WORKER_BINDING_SCHEMA.to_owned(),
            schema_version: WORKER_BINDING_SCHEMA_VERSION,
            binding_id: Id::new("worker_binding:shell-worker-test").expect("binding id"),
            worker_kind: WorkerKind::Shell,
            command: "/bin/sh".to_owned(),
            args: vec!["-c".to_owned(), script.to_owned()],
            working_directory: directory.display().to_string(),
            resolved_command_path: "/bin/sh".to_owned(),
            resolved_working_directory: directory.display().to_string(),
            command_content_hash: "0".repeat(64),
            env_allowlist: Vec::new(),
            timeout_ms,
            capability_ids: vec![Id::new("capability:shell-worker-test").expect("capability id")],
            metadata: Map::new(),
        };
        let identity = crate::exec::binding::resolve_worker_binding_identity(&binding)
            .expect("binding identity");
        binding.resolved_command_path = identity.resolved_command_path;
        binding.resolved_working_directory = identity.resolved_working_directory;
        binding.command_content_hash = identity.command_content_hash;
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
