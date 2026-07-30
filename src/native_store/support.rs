use super::{CaseSpace, MorphismLogEntry, NativeStoreError, NativeStoreResult};
use higher_graphen_core::Id;
use serde::Serialize;
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const LOCK_RETRY_ATTEMPTS: u32 = 8;
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
        let mut attempt = 0;
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
                    if attempt == LOCK_RETRY_ATTEMPTS {
                        return Err(NativeStoreError::LockUnavailable {
                            path,
                            reason: format!(
                                "exclusive native case-space lock remained held after {} attempts",
                                LOCK_RETRY_ATTEMPTS + 1
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
