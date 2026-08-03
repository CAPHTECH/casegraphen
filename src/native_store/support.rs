use super::{CaseSpace, MorphismLogEntry, NativeStoreError, NativeStoreResult};
use higher_graphen_core::Id;
use serde::Serialize;
use std::{
    fs::{self, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

/// How long to wait for the lock, as a deadline rather than a retry count.
///
/// The hold time scales with the case space — the lock is held across the
/// evaluator's contract check, the snapshot, the append and the head write —
/// while a fixed eight attempts capped at 40 ms was about 235 ms of patience.
/// Measured: one gated `cell transition` on a 4,000-cell space takes 3.0 s, so
/// two ordinary concurrent writers were guaranteed to fail one of them. That
/// is not the adversarial lock denial residual risk 8 describes; it is what a
/// large case space did to itself.
///
/// There is no staleness threshold to stay under any more (ADR 0017): the
/// tool never infers that another process is dead, so this budget bounds
/// patience alone. A waiter that reaches the deadline refuses with
/// `LockUnavailable` and leaves the lock file exactly as it found it.
const LOCK_WAIT_BUDGET: Duration = Duration::from_secs(30);
const LOCK_INITIAL_BACKOFF: Duration = Duration::from_millis(5);
const LOCK_MAX_BACKOFF: Duration = Duration::from_millis(40);
static LOCK_TOKEN_COUNTER: AtomicU64 = AtomicU64::new(1);

/// The one backoff computation `acquire`'s two "keep waiting" outcomes
/// share — a contended `AlreadyExists`, and the rarer race where this
/// iteration's own just-created file was gone by the time it read it back.
/// Both loop back to the same top-of-loop deadline check; this only advances
/// `attempt` and sleeps.
fn lock_backoff(attempt: &mut u32) {
    let multiplier = 1_u32 << (*attempt).min(3);
    thread::sleep(
        LOCK_INITIAL_BACKOFF
            .saturating_mul(multiplier)
            .min(LOCK_MAX_BACKOFF),
    );
    *attempt += 1;
}

pub(super) struct CaseLockGuard {
    path: PathBuf,
    lock_contents: String,
}

impl CaseLockGuard {
    pub(super) fn acquire(case_directory: &Path) -> NativeStoreResult<Self> {
        let path = case_directory.join(".lock");
        let ownership_token = lock_ownership_token();
        let lock_contents = format!("token={ownership_token}\n");
        let mut attempt = 0_u32;
        let deadline = Instant::now() + LOCK_WAIT_BUDGET;
        loop {
            // Checked at the top of every iteration, not only after
            // `AlreadyExists`: the read-back's own `NotFound` arm below also
            // loops back here without a guaranteed wait, and before this fix
            // it re-entered the `match` directly, bypassing the deadline
            // entirely — an unbounded, sleep-free spin on that path.
            if Instant::now() >= deadline {
                return Err(NativeStoreError::LockUnavailable {
                    path,
                    reason: format!(
                        "another process is writing to this case space; the exclusive lock \
                         remained held for {}s. This tool never infers that a live lock is \
                         abandoned (ADR 0017), so waiting longer will not resolve this on its \
                         own. If the holder has crashed, removing this lock file is the human \
                         assertion that it is gone — confirm that externally, then remove the \
                         file and retry",
                        LOCK_WAIT_BUDGET.as_secs()
                    ),
                });
            }
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(mut file) => {
                    if let Err(source) = file
                        .write_all(lock_contents.as_bytes())
                        .and_then(|()| file.flush())
                    {
                        drop(file);
                        let _ = fs::remove_file(&path);
                        return Err(NativeStoreError::Io {
                            path: path.clone(),
                            source,
                        });
                    }
                    drop(file);
                    match fs::read_to_string(&path) {
                        Ok(actual) if actual == lock_contents => {
                            return Ok(Self {
                                path,
                                lock_contents,
                            });
                        }
                        Ok(_) => {
                            return Err(NativeStoreError::LockUnavailable {
                                path,
                                reason:
                                    "native case-space lock ownership changed during acquisition"
                                        .to_owned(),
                            });
                        }
                        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
                            // Something removed the file this iteration just
                            // created between the write and this read — the
                            // top-of-loop deadline check above still bounds
                            // this path; back off the same as a contended
                            // `AlreadyExists` before retrying.
                            lock_backoff(&mut attempt);
                            continue;
                        }
                        Err(source) => {
                            // This branch owns a lock file it created and is
                            // about to abandon without constructing a guard,
                            // so no `Drop` will ever clean it up. The
                            // `write_all`/`flush` failure arm above already
                            // removes its own file for that reason; this one
                            // did not, and before ADR 0017 the asymmetry was
                            // survivable because the staleness check reclaimed
                            // the orphan after 60s. With that check gone the
                            // orphan is permanent, and it is indistinguishable
                            // from a live holder to every future waiter.
                            //
                            // Removed through `remove_lock_if_owned` rather
                            // than `fs::remove_file`, because the read that
                            // just failed is exactly the check for "did
                            // someone replace this file" — we cannot know from
                            // here that the file is still ours, and removing
                            // another process's lock is the thing ADR 0017
                            // forbids. The compare-and-delete answers that
                            // question, and leaves the file alone when it
                            // cannot.
                            let _ = remove_lock_if_owned(&path, &lock_contents);
                            return Err(NativeStoreError::Io {
                                path: path.clone(),
                                source,
                            });
                        }
                    }
                }
                Err(source) if source.kind() == std::io::ErrorKind::AlreadyExists => {
                    lock_backoff(&mut attempt);
                }
                Err(source) => {
                    return Err(NativeStoreError::Io {
                        path: path.clone(),
                        source,
                    });
                }
            }
        }
    }

    /// The ADR 0017 2026-08-02 amendment: confirms this process still holds
    /// the lock it acquired, immediately before a durable write. This is not
    /// a liveness inference about anyone else — it is the tool establishing
    /// that *it* still holds what it acquired, the one question it can
    /// actually answer about itself. `docs/specs/case-lock.fsl`'s `commit`
    /// action models this as the `lock == some(p)` guard on the write.
    ///
    /// Not atomic with the write that follows (`ASSUME-LOCK-001` in that
    /// spec): a TOCTOU window remains between this read and the write it
    /// guards. The window shrinks from the whole operation — 3.0 s measured
    /// on a 4,000-cell space — to microseconds. That is a reduction, not a
    /// closure, and no caller may treat this as making the write atomic.
    pub(super) fn still_owned(&self) -> NativeStoreResult<()> {
        #[cfg(test)]
        apply_pending_test_lock_displacement(&self.path);
        let owned = lock_file_matches(&self.path, &self.lock_contents).map_err(|source| {
            NativeStoreError::Io {
                path: self.path.clone(),
                source,
            }
        })?;
        if owned {
            return Ok(());
        }
        Err(NativeStoreError::LockUnavailable {
            path: self.path.clone(),
            reason: "another process removed or replaced this case-space lock before this \
                     process's durable write; refusing rather than writing under a lock it no \
                     longer holds"
                .to_owned(),
        })
    }
}

// Test-only seam for deterministically reproducing ADR 0017's displacement
// race — an operator's `rm` immediately followed by a new holder's
// `create_new` — landing exactly inside the window `still_owned` exists to
// close. A real race is timing-dependent and only fails under load, which is
// exactly the class of test this codebase is working to eliminate elsewhere
// (issue #32); this hook lets a test arm the *next* `still_owned` check on a
// named lock path to observe a foreign token, synchronously and without a
// second thread, so a test can drive the real production entry point
// (`append_morphism`, `rebuild_case_space_inner`) instead of hand-assembling
// `still_owned`'s call sequence.
#[cfg(test)]
thread_local! {
    static DISPLACE_LOCK_BEFORE_NEXT_STILL_OWNED_CHECK: std::cell::RefCell<Option<PathBuf>> =
        const { std::cell::RefCell::new(None) };
}

/// Arms the seam above: the next `still_owned` check on `lock_path` writes a
/// foreign ownership token to it first, so that check reads the lock as no
/// longer this process's own. One-shot — cleared on first match, so only the
/// next check on this path is affected, not every one after it.
#[cfg(test)]
pub(super) fn arrange_lock_displacement_before_next_still_owned_check(lock_path: PathBuf) {
    DISPLACE_LOCK_BEFORE_NEXT_STILL_OWNED_CHECK.with(|cell| {
        *cell.borrow_mut() = Some(lock_path);
    });
}

#[cfg(test)]
fn apply_pending_test_lock_displacement(path: &Path) {
    let armed = DISPLACE_LOCK_BEFORE_NEXT_STILL_OWNED_CHECK.with(|cell| {
        let mut pending = cell.borrow_mut();
        if pending.as_deref() == Some(path) {
            *pending = None;
            true
        } else {
            false
        }
    });
    if armed {
        let _ = fs::write(path, "token=foreign-displacement-from-test-hook\n");
    }
}

impl Drop for CaseLockGuard {
    fn drop(&mut self) {
        // Not reported: a `Drop` impl has no notion of `--format` and no
        // way to know whether this invocation is about to print a JSON
        // refusal to the same stream. A failure here means a `.lock` file
        // may outlive this process; the next acquire attempt surfaces that
        // as `LockUnavailable` on its own, which is the actionable form of
        // this failure, not a line here.
        let _ = remove_lock_if_owned(&self.path, &self.lock_contents);
    }
}

fn lock_ownership_token() -> String {
    let counter = LOCK_TOKEN_COUNTER.fetch_add(1, Ordering::Relaxed);
    let unix_nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!(
        "pid={}-counter={counter}-unix_nanos={unix_nanos}",
        std::process::id()
    )
}

/// "Is this lock still mine": the one predicate `still_owned` (the pre-write
/// check) and `remove_lock_if_owned` (the compare-and-delete on release)
/// both answer, extracted so it exists once rather than as two reads that
/// could drift. A missing file and a file carrying a different token are
/// both `Ok(false)` — either way, `token` no longer names the current state
/// of the lock.
fn lock_file_matches(path: &Path, token: &str) -> std::io::Result<bool> {
    match fs::read_to_string(path) {
        Ok(actual) => Ok(actual == token),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(source),
    }
}

fn remove_lock_if_owned(path: &Path, ownership_token: &str) -> std::io::Result<bool> {
    if !lock_file_matches(path, ownership_token)? {
        return Ok(false);
    }
    match fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(source),
    }
}

pub(super) fn parse_log_entries(
    path: &Path,
    text: &str,
) -> NativeStoreResult<Vec<MorphismLogEntry>> {
    let mut entries = Vec::new();
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        entries.push(
            serde_json::from_str(line).map_err(|source| NativeStoreError::Json {
                path: path.to_owned(),
                source,
            })?,
        );
    }
    Ok(entries)
}

pub(super) fn append_json_line(path: &Path, value: &impl Serialize) -> NativeStoreResult<()> {
    let text = serde_json::to_string(value).map_err(|source| NativeStoreError::Json {
        path: path.to_owned(),
        source,
    })?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|source| NativeStoreError::Io {
            path: path.to_owned(),
            source,
        })?;
    let previous_len = file
        .metadata()
        .map_err(|source| NativeStoreError::Io {
            path: path.to_owned(),
            source,
        })?
        .len();
    if let Err(source) = file.write_all(format!("{text}\n").as_bytes()) {
        file.set_len(previous_len)
            .map_err(|rollback_source| NativeStoreError::Io {
                path: path.to_owned(),
                source: rollback_source,
            })?;
        return Err(NativeStoreError::Io {
            path: path.to_owned(),
            source,
        });
    }
    Ok(())
}

/// Takes the case lock as a parameter and checks it (ADR 0017's 2026-08-02
/// amendment) before doing anything durable, rather than trusting a
/// hand-placed `lock.still_owned()?` at each call site to have been added —
/// issue #36: on the first application of that obligation, three of six
/// call sites were missed. A caller cannot reach this write without a
/// `CaseLockGuard` in hand; whether that guard still owns the lock is
/// checked here, not left to be remembered.
pub(super) fn append_verified_log_entry(
    lock: &CaseLockGuard,
    path: &Path,
    entry: &MorphismLogEntry,
) -> NativeStoreResult<u64> {
    lock.still_owned()?;
    let previous_len = fs::metadata(path)
        .map_err(|source| NativeStoreError::Io {
            path: path.to_owned(),
            source,
        })?
        .len();
    append_json_line(path, entry)?;

    let verification = (|| {
        let mut file =
            OpenOptions::new()
                .read(true)
                .open(path)
                .map_err(|source| NativeStoreError::Io {
                    path: path.to_owned(),
                    source,
                })?;
        if previous_len > 0 {
            file.seek(SeekFrom::Start(previous_len - 1))
                .and_then(|_| {
                    let mut delimiter = [0_u8; 1];
                    file.read_exact(&mut delimiter)?;
                    if delimiter[0] != b'\n' {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            "morphism log did not end with a newline before append",
                        ));
                    }
                    Ok(())
                })
                .map_err(|source| NativeStoreError::Io {
                    path: path.to_owned(),
                    source,
                })?;
        }
        file.seek(SeekFrom::Start(previous_len))
            .map_err(|source| NativeStoreError::Io {
                path: path.to_owned(),
                source,
            })?;
        let mut appended = String::new();
        file.read_to_string(&mut appended)
            .map_err(|source| NativeStoreError::Io {
                path: path.to_owned(),
                source,
            })?;
        if !appended.ends_with('\n') || appended[..appended.len() - 1].contains('\n') {
            return Err(NativeStoreError::ReplayMismatch {
                path: path.to_owned(),
                reason: "appended morphism log entry is not exactly one JSON line".to_owned(),
            });
        }
        let actual: MorphismLogEntry = serde_json::from_str(appended.trim_end_matches('\n'))
            .map_err(|source| NativeStoreError::Json {
                path: path.to_owned(),
                source,
            })?;
        let actual_hash =
            crate::native_hash::morphism_log_entry_hash(&actual).map_err(|source| {
                NativeStoreError::Json {
                    path: path.to_owned(),
                    source,
                }
            })?;
        let expected_hash =
            crate::native_hash::morphism_log_entry_hash(entry).map_err(|source| {
                NativeStoreError::Json {
                    path: path.to_owned(),
                    source,
                }
            })?;
        if actual_hash != expected_hash {
            return Err(NativeStoreError::ReplayMismatch {
                path: path.to_owned(),
                reason: format!(
                    "appended morphism log entry hash {actual_hash} does not match expected {expected_hash}"
                ),
            });
        }
        Ok(())
    })();

    if let Err(error) = verification {
        truncate_after_failed_append(path, previous_len, &error)?;
        return Err(error);
    }
    Ok(previous_len)
}

pub(super) fn truncate_after_failed_append(
    path: &Path,
    previous_len: u64,
    append_error: &NativeStoreError,
) -> NativeStoreResult<()> {
    OpenOptions::new()
        .write(true)
        .open(path)
        .and_then(|file| file.set_len(previous_len))
        .map_err(|source| NativeStoreError::ReplayMismatch {
            path: path.to_owned(),
            reason: format!(
                "failed to roll back morphism log after {append_error}; truncation failed: {source}"
            ),
        })
}

/// The unchecked `create_new`-a-JSON-file implementation. Private: nothing
/// outside this module may call it directly, and within this module only the
/// two functions below do — `write_json_create_new_owned` (checked, the only
/// path production code has) and, only under `#[cfg(test)]`,
/// `write_json_create_new_without_lock_check` (the escape hatch a test uses
/// to write a snapshot as a legacy or foreign writer would, one that never
/// held this store's lock at all). Issue #36: a `pub(super)` raw function
/// sitting next to a checked wrapper is still a call-site obligation with a
/// longer name — a future write path could still compile a call to the raw
/// one. Keeping it private and reaching it only through those two names
/// means a production write path has nothing unchecked left to call.
fn write_json_create_new_impl(path: &Path, value: &impl Serialize) -> NativeStoreResult<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| NativeStoreError::Io {
            path: parent.to_owned(),
            source,
        })?;
    }
    let text = serde_json::to_string_pretty(value).map_err(|source| NativeStoreError::Json {
        path: path.to_owned(),
        source,
    })?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|source| NativeStoreError::Io {
            path: path.to_owned(),
            source,
        })?;
    if let Err(source) = writeln!(file, "{text}") {
        drop(file);
        let _ = fs::remove_file(path);
        return Err(NativeStoreError::Io {
            path: path.to_owned(),
            source,
        });
    }
    Ok(())
}

/// The lock-checked variant of `write_json_create_new_impl` (ADR 0017's
/// 2026-08-02 amendment, issue #36): every in-process durable snapshot write
/// takes the guard it was written under and confirms it still owns the lock
/// immediately before writing, rather than relying on a hand-placed
/// `lock.still_owned()?` at the call site that a future write path could
/// omit. This is the only way production code can reach the implementation.
pub(super) fn write_json_create_new_owned(
    lock: &CaseLockGuard,
    path: &Path,
    value: &impl Serialize,
) -> NativeStoreResult<()> {
    lock.still_owned()?;
    write_json_create_new_impl(path, value)
}

/// Test-only escape hatch to the unchecked implementation, for fixtures that
/// simulate a legacy or foreign snapshot writer — one that never held this
/// store's lock at all, so it has no guard to check. `#[cfg(test)]` makes
/// this genuinely unreachable from production code, not merely unlisted in
/// some allowlist: the symbol does not exist in a non-test build, so a new
/// production write path has no unchecked name left to call by mistake.
#[cfg(test)]
pub(super) fn write_json_create_new_without_lock_check(
    path: &Path,
    value: &impl Serialize,
) -> NativeStoreResult<()> {
    write_json_create_new_impl(path, value)
}

pub(super) fn latest_entry<'a>(
    entries: &'a [MorphismLogEntry],
    path: &Path,
) -> NativeStoreResult<&'a MorphismLogEntry> {
    entries
        .last()
        .ok_or_else(|| NativeStoreError::ReplayMismatch {
            path: path.to_owned(),
            reason: "morphism log is empty".to_owned(),
        })
}

pub(super) fn require_relative_store_path(path: &Path, value: &str) -> NativeStoreResult<()> {
    let candidate = Path::new(value);
    if value.trim().is_empty() {
        return Err(NativeStoreError::ReplayMismatch {
            path: path.to_owned(),
            reason: "snapshot path is empty".to_owned(),
        });
    }
    for component in candidate.components() {
        if !matches!(component, Component::Normal(_)) {
            return Err(NativeStoreError::ReplayMismatch {
                path: path.to_owned(),
                reason: format!("snapshot path {value:?} must stay inside the native store"),
            });
        }
    }
    Ok(())
}

pub(super) fn case_space_checksum(case_space: &CaseSpace) -> NativeStoreResult<String> {
    crate::native_hash::case_space_checksum(case_space).map_err(|source| NativeStoreError::Json {
        path: PathBuf::from("<case-space-checksum>"),
        source,
    })
}

pub(super) fn path_segment(id: &Id) -> String {
    let mut segment = String::new();
    for byte in id.as_str().bytes() {
        match byte {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' => {
                segment.push(byte as char);
            }
            _ => segment.push_str(&format!("~{byte:02x}")),
        }
    }
    segment
}

pub(super) fn invalid_morphism(path: &Path, reason: impl Into<String>) -> NativeStoreError {
    NativeStoreError::InvalidMorphism {
        path: path.to_owned(),
        reason: reason.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::FileTimes;

    fn lock_guard_test_dir(label: &str) -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
        let unix_nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "casegraphen-case-lock-guard-{label}-{}-{unix_nanos}-{counter}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).expect("create case-lock-guard test directory");
        dir
    }

    /// ADR 0017 / issue #30: an aged lock file is not evidence the holder is
    /// dead, so `acquire` must not break it. This waits out the real
    /// `LOCK_WAIT_BUDGET` on purpose, the same choice
    /// `append_fails_while_case_lock_is_held_without_corrupting_history` in
    /// `native_store/tests.rs` makes: shrinking the constant to make the
    /// test faster would stop testing the timing the refusal actually
    /// depends on.
    #[test]
    fn acquire_refuses_an_aged_lock_and_leaves_it_byte_identical() {
        let dir = lock_guard_test_dir("aged-refuse");
        let lock_path = dir.join(".lock");
        let forged_contents = "token=forged-aged-lock\n";
        fs::write(&lock_path, forged_contents).expect("forge a lock file");
        OpenOptions::new()
            .write(true)
            .open(&lock_path)
            .expect("open forged lock")
            .set_times(FileTimes::new().set_modified(UNIX_EPOCH))
            .expect("age forged lock far past the old 60s staleness threshold");

        let error = match CaseLockGuard::acquire(&dir) {
            Err(error) => error,
            Ok(_guard) => panic!("an aged lock must not be acquired"),
        };

        assert!(
            matches!(error, NativeStoreError::LockUnavailable { .. }),
            "unexpected error: {error:?}"
        );
        assert!(
            lock_path.exists(),
            "refusing to acquire must not remove the lock file"
        );
        assert_eq!(
            fs::read_to_string(&lock_path).expect("read lock file after refusal"),
            forged_contents,
            "the lock file must be byte-identical after a refusal"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    /// The property `docs/specs/case-lock.fsl` proves
    /// (`MODEL-LOCK-009`/`INV-LOCK-001`): nothing removes or replaces a lock
    /// file that is not the exact token it was given. Driving genuine OS
    /// concurrency here would mean real threads racing on `create_new`, and
    /// a contended `acquire` waits out the real 30 s `LOCK_WAIT_BUDGET`
    /// before giving up — fine for one deterministic test, impractical for
    /// an arbtest fuzz loop. So this drives the real `CaseLockGuard::acquire`
    /// (only ever called when the model already knows the file is free, so
    /// it always succeeds immediately) and the real `Drop`
    /// (`remove_lock_if_owned`) against one real directory, over an
    /// arbtest-chosen sequence of acquire / foreign-write / release steps
    /// applied one at a time by this single thread. `ForeignWrite` stands in
    /// for a second process's lock file — something no two real `acquire`
    /// calls in one process could produce concurrently, since `create_new`
    /// is atomic — and is exactly the scenario the decision must survive: a
    /// live guard's own `Drop` must never remove or replace a lock file
    /// whose on-disk content is not the token that guard was given.
    ///
    /// This is real concurrency of neither timing nor OS processes — it is
    /// a real `CaseLockGuard` driven through an arbitrary *interleaving of
    /// operations* against one directory, which is the property the FSL
    /// model states. It does not exercise the wait-then-refuse timing path;
    /// `acquire_refuses_an_aged_lock_and_leaves_it_byte_identical` above
    /// covers that deterministically instead.
    #[test]
    fn a_live_guards_lock_file_is_never_removed_or_replaced_by_another_acquirer() {
        // Coverage witness (adversarial-review finding on this test itself):
        // `Step::AcquireIfFree` skips whenever `on_disk.is_some()`, and
        // nothing in this model ever clears a foreign write back to `None`
        // except a genuine own-guard release. So a scenario that opens with
        // `ForeignWrite` can spend every remaining step as a no-op `continue`
        // and assert nothing at all — this repo has already shipped one test
        // that silently degraded to asserting nothing, and this must not be
        // a second. Counts executions of the "not owned" branch (the one
        // that actually exercises displacement) across the whole arbtest
        // run, asserted `> 0` afterwards.
        let displaced_release_witnessed = std::cell::Cell::new(0_usize);
        arbtest::arbtest(
            |u: &mut arbtest::arbitrary::Unstructured<'_>| -> arbtest::arbitrary::Result<()> {
                let dir = lock_guard_test_dir("pbt-interleaving");
                let lock_path = dir.join(".lock");

                // Model state, tracked alongside the real filesystem: the
                // current lock file's real content, if any, and the live
                // guard object when the last acquire succeeded and has not
                // yet been dropped.
                let mut on_disk: Option<String> = None;
                let mut live_guard: Option<CaseLockGuard> = None;

                #[derive(Clone, Copy)]
                enum Step {
                    AcquireIfFree,
                    ForeignWrite,
                    Release,
                }

                let step_count = u.int_in_range(1_u8..=8)?;
                for _ in 0..step_count {
                    let step =
                        *u.choose(&[Step::AcquireIfFree, Step::ForeignWrite, Step::Release])?;
                    match step {
                        Step::AcquireIfFree => {
                            if on_disk.is_some() {
                                // A real acquire() here would block for up
                                // to LOCK_WAIT_BUDGET; skip rather than pay
                                // that cost every iteration.
                                continue;
                            }
                            let guard = CaseLockGuard::acquire(&dir).expect("acquire a free lock");
                            let contents = fs::read_to_string(&lock_path)
                                .expect("read just-acquired lock file");
                            on_disk = Some(contents);
                            live_guard = Some(guard);
                        }
                        Step::ForeignWrite => {
                            let token_bytes: Vec<u8> = u.arbitrary()?;
                            let token_hex = token_bytes.iter().fold(
                                String::with_capacity(token_bytes.len() * 2),
                                |mut output, byte| {
                                    use std::fmt::Write as _;
                                    write!(&mut output, "{byte:02x}")
                                        .expect("writing to a String cannot fail");
                                    output
                                },
                            );
                            let forged = format!("token=foreign-{token_hex}\n");
                            fs::write(&lock_path, &forged)
                                .expect("forge a foreign lock file's content");
                            on_disk = Some(forged);
                            // The live guard, if any, still believes it owns
                            // its original token — it is never told.
                        }
                        Step::Release => {
                            let Some(guard) = live_guard.take() else {
                                continue;
                            };
                            let owned_token = guard.lock_contents.clone();
                            let before_drop = on_disk.clone();
                            drop(guard);
                            let after_drop = fs::read_to_string(&lock_path).ok();
                            if before_drop.as_deref() == Some(owned_token.as_str()) {
                                assert!(
                                    after_drop.is_none(),
                                    "a guard must remove its own untouched lock file on drop"
                                );
                                on_disk = None;
                            } else {
                                displaced_release_witnessed
                                    .set(displaced_release_witnessed.get() + 1);
                                assert_eq!(
                                    after_drop, before_drop,
                                    "a guard must never remove or replace a lock file \
                                     it does not own"
                                );
                            }
                        }
                    }
                }

                if let Some(guard) = live_guard.take() {
                    let owned_token = guard.lock_contents.clone();
                    drop(guard);
                    let after_drop = fs::read_to_string(&lock_path).ok();
                    if on_disk.as_deref() == Some(owned_token.as_str()) {
                        assert!(after_drop.is_none());
                    } else {
                        displaced_release_witnessed.set(displaced_release_witnessed.get() + 1);
                        assert_eq!(after_drop, on_disk);
                    }
                }

                let _ = fs::remove_dir_all(&dir);
                Ok(())
            },
        );
        assert!(
            displaced_release_witnessed.get() > 0,
            "the whole arbtest run never exercised a displaced release — this property test \
             would pass even if displacement handling were broken"
        );
    }
}
