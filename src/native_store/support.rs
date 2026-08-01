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
/// Kept well under `LOCK_STALE_AFTER` so a waiter gives up before it could
/// mistake a live holder for an abandoned one.
const LOCK_WAIT_BUDGET: Duration = Duration::from_secs(30);
const LOCK_INITIAL_BACKOFF: Duration = Duration::from_millis(5);
const LOCK_MAX_BACKOFF: Duration = Duration::from_millis(40);
const LOCK_STALE_AFTER: Duration = Duration::from_secs(60);
static LOCK_TOKEN_COUNTER: AtomicU64 = AtomicU64::new(1);

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
                        Err(source) if source.kind() == std::io::ErrorKind::NotFound => continue,
                        Err(source) => {
                            return Err(NativeStoreError::Io {
                                path: path.clone(),
                                source,
                            });
                        }
                    }
                }
                Err(source) if source.kind() == std::io::ErrorKind::AlreadyExists => {
                    let observed_token = match fs::read_to_string(&path) {
                        Ok(token) => token,
                        Err(source) if source.kind() == std::io::ErrorKind::NotFound => continue,
                        Err(source) => {
                            return Err(NativeStoreError::Io {
                                path: path.clone(),
                                source,
                            });
                        }
                    };
                    let metadata = match fs::metadata(&path) {
                        Ok(metadata) => metadata,
                        Err(source) if source.kind() == std::io::ErrorKind::NotFound => continue,
                        Err(source) => {
                            return Err(NativeStoreError::Io {
                                path: path.clone(),
                                source,
                            });
                        }
                    };
                    let stale = metadata
                        .modified()
                        .ok()
                        .and_then(|modified| SystemTime::now().duration_since(modified).ok())
                        .is_some_and(|age| age >= LOCK_STALE_AFTER);
                    if stale {
                        match remove_lock_if_owned(&path, &observed_token) {
                            Ok(true) => {
                                eprintln!(
                                    "{}: broke stale native case-space lock older than {} seconds",
                                    path.display(),
                                    LOCK_STALE_AFTER.as_secs()
                                );
                                continue;
                            }
                            Ok(false) => {}
                            Err(source) => {
                                return Err(NativeStoreError::Io {
                                    path: path.clone(),
                                    source,
                                });
                            }
                        }
                    }
                    if Instant::now() >= deadline {
                        return Err(NativeStoreError::LockUnavailable {
                            path,
                            reason: format!(
                                "another process is writing to this case space; the exclusive \
                                 lock remained held for {}s. Re-read the current revision and \
                                 retry",
                                LOCK_WAIT_BUDGET.as_secs()
                            ),
                        });
                    }
                    let multiplier = 1_u32 << attempt.min(3);
                    thread::sleep(
                        LOCK_INITIAL_BACKOFF
                            .saturating_mul(multiplier)
                            .min(LOCK_MAX_BACKOFF),
                    );
                    attempt += 1;
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
}

impl Drop for CaseLockGuard {
    fn drop(&mut self) {
        if let Err(source) = remove_lock_if_owned(&self.path, &self.lock_contents) {
            eprintln!(
                "{}: failed to inspect or remove native case-space lock: {source}",
                self.path.display()
            );
        }
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

fn remove_lock_if_owned(path: &Path, ownership_token: &str) -> std::io::Result<bool> {
    let actual = match fs::read_to_string(path) {
        Ok(actual) => actual,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(source) => return Err(source),
    };
    if actual != ownership_token {
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

pub(super) fn append_verified_log_entry(
    path: &Path,
    entry: &MorphismLogEntry,
) -> NativeStoreResult<u64> {
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

pub(super) fn write_json_create_new(path: &Path, value: &impl Serialize) -> NativeStoreResult<()> {
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
