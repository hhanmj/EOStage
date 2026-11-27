//! B15 (2026-10-31 owner feedback): task-scoped, user-confirmed cleanup of the
//! conflict files the downloader renames aside (`<name>.part.eostage-conflict-<UTC>`,
//! `<name>.part.resume.json.eostage-conflict-<UTC>`, and the final-file
//! `<name>.eostage-conflict-<UTC>` the local reuse gate records).
//!
//! The Owner rule is unchanged: nothing is deleted silently. `list_conflict_files`
//! only enumerates what a task's own target directories hold; deletion happens
//! only through `delete_conflict_files` with the exact (path, size, mtime) the
//! user confirmed, and every file that no longer matches exactly is skipped,
//! never deleted. Anything resolving outside the task output root, symlinks,
//! junctions and directories are rejected outright.

use std::fs;
use std::path::{Component, Path, PathBuf};

use insar_core::{
    TaskConflictFile, TaskConflictFileDeleteOutcome, TaskConflictFileDeleteResult,
    TaskConflictFileList,
};

use crate::local_reuse::CONFLICT_MARKER;

/// One planned item target directory and the base file names whose conflict
/// copies may exist inside it (e.g. `a.zip` covers `a.zip.part.eostage-conflict-*`,
/// `a.zip.part.resume.json.eostage-conflict-*` and, when the Store recorded
/// it, `a.zip.eostage-conflict-*`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConflictTarget {
    pub directory: PathBuf,
    pub file_names: Vec<String>,
}

/// Why a conflict-cleanup request was refused before touching any file.
#[derive(Debug, PartialEq, Eq)]
pub enum ConflictCleanupError {
    /// The task output root itself is not a plain directory (symlink/junction).
    UnsafeRoot(PathBuf),
    /// A target directory is not a plain directory inside the root.
    UnsafeEntry(PathBuf),
    /// A Store-recorded conflict name is not a plain conflict file name.
    UnsafeRecordedName(String),
    /// A requested deletion path is outside the root or not conflict-shaped.
    UnsafeRequestedPath(String),
    /// Reading a directory or a file's metadata failed.
    Io(std::io::ErrorKind),
}

impl std::fmt::Display for ConflictCleanupError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsafeRoot(_) => write!(formatter, "task output root is not a plain directory"),
            Self::UnsafeEntry(_) => write!(formatter, "conflict entry is not a plain file"),
            Self::UnsafeRecordedName(_) => {
                write!(formatter, "recorded conflict name is not a plain file name")
            }
            Self::UnsafeRequestedPath(_) => {
                write!(
                    formatter,
                    "requested path is outside the task or not a conflict file"
                )
            }
            Self::Io(_) => write!(formatter, "filesystem read failed"),
        }
    }
}

impl std::error::Error for ConflictCleanupError {}

/// The typed per-file outcome of a confirmed deletion round.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConflictDeleteOutcome {
    Deleted,
    SkippedMissing,
    SkippedChanged,
    SkippedUnsafe,
}

impl ConflictDeleteOutcome {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Deleted => "deleted",
            Self::SkippedMissing => "skipped_missing",
            Self::SkippedChanged => "skipped_changed",
            Self::SkippedUnsafe => "skipped_unsafe",
        }
    }
}

/// True when `name` is `<base><stem>` where stem is
/// `.part.eostage-conflict-<stamp>` or `.part.resume.json.eostage-conflict-<stamp>`
/// (with the optional `-<attempt>` suffix the quarantine adds) and `<stamp>`
/// is a valid compact UTC stamp.
fn is_part_conflict_name(name: &str, base: &str) -> bool {
    is_conflict_with_stem(name, base, ".part", true)
        || is_conflict_with_stem(name, base, ".part.resume.json", true)
}

/// True when `name` is `<base>.eostage-conflict-<stamp>[-<attempt>]`: the
/// final-file conflict shape the local reuse gate records by name.
fn is_final_conflict_name(name: &str, base: &str) -> bool {
    is_conflict_with_stem(name, base, "", false)
}

fn is_conflict_with_stem(name: &str, base: &str, middle: &str, _part: bool) -> bool {
    let Some(rest) = name.strip_prefix(base) else {
        return false;
    };
    let Some(rest) = rest.strip_prefix(middle) else {
        return false;
    };
    let Some(stem) = rest.strip_prefix(CONFLICT_MARKER) else {
        return false;
    };
    // stem is `<stamp>` or `<stamp>-<attempt>`.
    let (stamp, attempt) = match stem.split_once('-') {
        Some((stamp, attempt)) => (stamp, Some(attempt)),
        None => (stem, None),
    };
    is_compact_utc_stamp(stamp)
        && attempt.is_none_or(|value| {
            !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit())
        })
}

/// 20261020T010203Z: eight date digits, `T`, six time digits, `Z`, with each
/// field inside its calendar range.
fn is_compact_utc_stamp(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 16 || bytes[8] != b'T' || bytes[15] != b'Z' {
        return false;
    }
    if !bytes[..8]
        .iter()
        .chain(&bytes[9..15])
        .all(|byte| byte.is_ascii_digit())
    {
        return false;
    }
    let digits = |range: std::ops::Range<usize>| -> u32 {
        bytes[range]
            .iter()
            .fold(0, |value, byte| value * 10 + u32::from(byte - b'0'))
    };
    let (year, month, day) = (digits(0..4), digits(4..6), digits(6..8));
    let (hour, minute, second) = (digits(9..11), digits(11..13), digits(13..15));
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let month_days = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    (1..=12).contains(&month)
        && day >= 1
        && day <= month_days[(month - 1) as usize]
        && hour <= 23
        && minute <= 59
        && second <= 60
}

/// A relative path with no `..`, no root, and no prefix components: it can
/// only name something inside the root it will be joined to.
fn is_bounded_relative(path: &str) -> bool {
    !path.is_empty()
        && path.chars().count() <= 4096
        && Path::new(path)
            .components()
            .all(|component| matches!(component, Component::Normal(_) | Component::CurDir))
}

/// A bounded relative path whose final component is exactly conflict-shaped
/// for one of `bases`.
fn is_conflict_relative(path: &str, bases: &[String]) -> bool {
    if !is_bounded_relative(path) {
        return false;
    }
    Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| {
            bases
                .iter()
                .any(|base| is_part_conflict_name(name, base) || is_final_conflict_name(name, base))
        })
}

/// A plain file name (no separators, no parent references) that is a recorded
/// conflict shape for one of `bases`.
fn is_recorded_conflict_name(name: &str, bases: &[String]) -> bool {
    is_bounded_relative(name)
        && !name.contains(['/', '\\'])
        && bases
            .iter()
            .any(|base| is_part_conflict_name(name, base) || is_final_conflict_name(name, base))
}

/// The metadata of an existing entry when it is a plain regular file; a
/// directory, symlink or junction is `None` for the caller to classify.
fn plain_file_metadata(path: &Path) -> Result<Option<fs::Metadata>, ConflictCleanupError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(ConflictCleanupError::Io(error.kind())),
    };
    if !metadata.is_file() {
        return Ok(None);
    }
    Ok(Some(metadata))
}

fn modified_epoch_ms(metadata: &fs::Metadata) -> u64 {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

/// Whether `directory` still resolves inside `root` once every link on the
/// way is followed — an intermediate junction must not lead outside the root.
/// A missing directory resolves nowhere and counts as inside (nothing to read).
fn resolves_inside_root(root: &Path, directory: &Path) -> Result<bool, ConflictCleanupError> {
    let root = fs::canonicalize(root).map_err(|error| ConflictCleanupError::Io(error.kind()))?;
    match fs::canonicalize(directory) {
        Ok(resolved) => Ok(resolved.starts_with(&root)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(true),
        Err(error) => Err(ConflictCleanupError::Io(error.kind())),
    }
}

/// A target directory must be a plain directory inside the root: a symlink,
/// junction or missing directory is refused before anything is read.
fn check_target_directory(root: &Path, directory: &Path) -> Result<(), ConflictCleanupError> {
    if !directory.starts_with(root) || !resolves_inside_root(root, directory)? {
        return Err(ConflictCleanupError::UnsafeEntry(directory.to_path_buf()));
    }
    match fs::symlink_metadata(directory) {
        Ok(metadata) if metadata.is_dir() => Ok(()),
        // A missing target directory simply has no conflict files.
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(ConflictCleanupError::Io(error.kind())),
        Ok(_) => Err(ConflictCleanupError::UnsafeEntry(directory.to_path_buf())),
    }
}

/// Enumerate the conflict files of one task. `root` is the task's persisted
/// output root; `targets` are the task's own item target directories and base
/// file names; `recorded` are the conflict file names the Store recorded as
/// renamed-aside local reuse decisions (plain names that only resolve inside
/// the target directories). The returned paths are relative to `root`.
pub fn list_conflict_files(
    root: &Path,
    targets: &[ConflictTarget],
    recorded: &[String],
) -> Result<TaskConflictFileList, ConflictCleanupError> {
    match fs::symlink_metadata(root) {
        Ok(metadata) if metadata.is_dir() => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound && recorded.is_empty() => {
            return Ok(TaskConflictFileList {
                task_id: String::new(),
                files: Vec::new(),
                total_count: 0,
                total_size_bytes: 0,
            });
        }
        Err(error) => return Err(ConflictCleanupError::Io(error.kind())),
        Ok(_) => return Err(ConflictCleanupError::UnsafeRoot(root.to_path_buf())),
    }
    let bases: Vec<String> = targets
        .iter()
        .flat_map(|target| target.file_names.iter().cloned())
        .collect();
    let mut recorded_names: Vec<&str> = Vec::new();
    for name in recorded {
        if !is_recorded_conflict_name(name, &bases) {
            return Err(ConflictCleanupError::UnsafeRecordedName(name.clone()));
        }
        recorded_names.push(name);
    }

    let mut files = Vec::new();
    for target in targets {
        check_target_directory(root, &target.directory)?;
        let entries = match fs::read_dir(&target.directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(ConflictCleanupError::Io(error.kind())),
        };
        for entry in entries {
            let entry = entry.map_err(|error| ConflictCleanupError::Io(error.kind()))?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let is_shape = target
                .file_names
                .iter()
                .any(|base| is_part_conflict_name(&name, base));
            let is_recorded = recorded_names.contains(&name.as_str());
            if !is_shape && !is_recorded {
                continue;
            }
            let path = entry.path();
            let Some(metadata) = plain_file_metadata(&path)? else {
                // A directory, symlink or junction named like a conflict file:
                // never listed, never deletable through this surface.
                continue;
            };
            let relative = path
                .strip_prefix(root)
                .map_err(|_| ConflictCleanupError::UnsafeEntry(path.clone()))?
                .to_string_lossy()
                .replace('/', std::path::MAIN_SEPARATOR_STR);
            files.push(TaskConflictFile {
                path: relative,
                size_bytes: metadata.len(),
                modified_at_epoch_ms: modified_epoch_ms(&metadata),
                recorded: is_recorded,
            });
        }
    }
    files.sort_by(|left, right| left.path.cmp(&right.path));
    let total_count = files.len() as u64;
    let total_size_bytes = files.iter().map(|file| file.size_bytes).sum();
    Ok(TaskConflictFileList {
        task_id: String::new(),
        files,
        total_count,
        total_size_bytes,
    })
}

/// Delete confirmed conflict files. Each expectation is the exact
/// (relative path, size, mtime) the user saw; a file is deleted only when it
/// still matches exactly, stays inside the root and keeps the conflict shape.
/// Missing, changed or unsafe entries are skipped with a typed outcome, so a
/// retry of the same request is idempotent.
pub fn delete_conflict_files(
    root: &Path,
    targets: &[ConflictTarget],
    expected: &[(String, u64, u64)],
) -> Result<TaskConflictFileDeleteResult, ConflictCleanupError> {
    match fs::symlink_metadata(root) {
        Ok(metadata) if metadata.is_dir() => {}
        Err(error) => return Err(ConflictCleanupError::Io(error.kind())),
        Ok(_) => return Err(ConflictCleanupError::UnsafeRoot(root.to_path_buf())),
    }
    let bases: Vec<String> = targets
        .iter()
        .flat_map(|target| target.file_names.iter().cloned())
        .collect();
    let mut outcomes = Vec::new();
    for (relative, expected_size, expected_mtime) in expected {
        if !is_conflict_relative(relative, &bases) {
            return Err(ConflictCleanupError::UnsafeRequestedPath(relative.clone()));
        }
        let path = root.join(relative);
        // The file must sit directly in one of the task's own target
        // directories, and that directory must resolve inside the root.
        let in_target = path
            .parent()
            .is_some_and(|parent| targets.iter().any(|target| target.directory == parent));
        if !path.starts_with(root) || !in_target {
            return Err(ConflictCleanupError::UnsafeRequestedPath(relative.clone()));
        }
        if let Some(parent) = path.parent() {
            if !resolves_inside_root(root, parent)? {
                return Err(ConflictCleanupError::UnsafeRequestedPath(relative.clone()));
            }
        }
        let outcome = match plain_file_metadata(&path)? {
            // Missing, or a directory/symlink/junction where a plain conflict
            // file was confirmed: skip, never delete.
            None => ConflictDeleteOutcome::SkippedUnsafe,
            Some(metadata) => {
                if metadata.len() != *expected_size
                    || modified_epoch_ms(&metadata) != *expected_mtime
                {
                    ConflictDeleteOutcome::SkippedChanged
                } else {
                    match fs::remove_file(&path) {
                        Ok(()) => ConflictDeleteOutcome::Deleted,
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                            ConflictDeleteOutcome::SkippedMissing
                        }
                        Err(_) => ConflictDeleteOutcome::SkippedUnsafe,
                    }
                }
            }
        };
        outcomes.push(TaskConflictFileDeleteOutcome {
            path: relative.clone(),
            outcome: outcome.as_str().to_owned(),
        });
    }
    Ok(TaskConflictFileDeleteResult {
        task_id: String::new(),
        outcomes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conflict_shapes_are_recognized_exactly() {
        assert!(is_part_conflict_name(
            "a.zip.part.eostage-conflict-20261020T010203Z",
            "a.zip"
        ));
        assert!(is_part_conflict_name(
            "a.zip.part.eostage-conflict-20261020T010203Z-2",
            "a.zip"
        ));
        assert!(is_part_conflict_name(
            "a.zip.part.resume.json.eostage-conflict-20261020T010203Z",
            "a.zip"
        ));
        assert!(is_final_conflict_name(
            "a.zip.eostage-conflict-20261020T010203Z",
            "a.zip"
        ));
        assert!(!is_part_conflict_name(
            "a.zip.eostage-conflict-20261020T010203Z",
            "a.zip"
        ));
        assert!(!is_part_conflict_name(
            "a.zip.part.eostage-conflict-20261020T010203Z-tail",
            "a.zip"
        ));
        assert!(!is_part_conflict_name(
            "prefix.a.zip.part.eostage-conflict-20261020T010203Z",
            "a.zip"
        ));
        assert!(!is_part_conflict_name(
            "a.zip.part.eostage-conflict-20261020010203",
            "a.zip"
        ));
        assert!(!is_part_conflict_name(
            "a.zip.part.eostage-conflict-20261132T010203Z",
            "a.zip"
        ));
        assert!(!is_part_conflict_name(
            "a.zip.part.eostage-conflict-20261020T240203Z",
            "a.zip"
        ));
    }
}
