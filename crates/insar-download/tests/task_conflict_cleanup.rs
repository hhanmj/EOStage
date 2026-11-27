//! B15 (2026-10-31 owner feedback): task-scoped, user-confirmed cleanup of the
//! conflict files the downloader renames aside. The Owner rule is unchanged —
//! nothing is ever deleted silently; these functions only enumerate and, after
//! the user confirms, delete files that still match exactly what was listed.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use insar_download::conflict_cleanup::{
    delete_conflict_files, list_conflict_files, ConflictCleanupError, ConflictTarget,
};

fn unique_root(label: &str) -> PathBuf {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("insar-b15-{label}-{suffix}"));
    fs::create_dir_all(&root).unwrap();
    root
}

/// One planned item target directory plus the base file name whose conflict
/// copies may exist for it.
fn target(directory: &Path, file_name: &str) -> ConflictTarget {
    ConflictTarget {
        directory: directory.to_path_buf(),
        file_names: vec![file_name.to_owned()],
    }
}

fn write_file(path: &Path, bytes: &[u8]) -> u64 {
    fs::write(path, bytes).unwrap();
    bytes.len() as u64
}

fn mtime_ms(path: &Path) -> u64 {
    fs::metadata(path)
        .unwrap()
        .modified()
        .unwrap()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

fn set_mtime(path: &Path, epoch_ms: u64) {
    let time = filetime::FileTime::from_unix_time(
        (epoch_ms / 1000) as i64,
        ((epoch_ms % 1000) * 1_000_000) as u32,
    );
    filetime::set_file_mtime(path, time).unwrap();
}

#[test]
fn lists_only_exact_conflict_shapes() {
    let root = unique_root("list-shape");
    let real = root.join("a.zip.part.eostage-conflict-20261020T000000Z");
    let resume = root.join("a.zip.part.resume.json.eostage-conflict-20261020T000000Z");
    let recorded_final = root.join("a.zip.eostage-conflict-20261020T000000Z");
    let decoy_marker_only = root.join("not-a-conflict.eostage-conflict");
    let decoy_prefix = root.join("prefix-a.zip.part.eostage-conflict-20261020T000000Z");
    let decoy_suffix = root.join("a.zip.part.eostage-conflict-20261020T000000Z-tail");
    let decoy_bad_stamp = root.join("a.zip.part.eostage-conflict-20261132T000000Z");
    let plain = root.join("a.zip");
    write_file(&real, b"part-bytes");
    write_file(&resume, b"{}");
    write_file(&recorded_final, b"zip");
    write_file(&decoy_marker_only, b"x");
    write_file(&decoy_prefix, b"x");
    write_file(&decoy_suffix, b"x");
    write_file(&decoy_bad_stamp, b"x");
    write_file(&plain, b"zip");

    let listed = list_conflict_files(
        &root,
        &[target(&root, "a.zip")],
        &["a.zip.eostage-conflict-20261020T000000Z".to_owned()],
    )
    .unwrap();

    let paths: Vec<&str> = listed.files.iter().map(|file| file.path.as_str()).collect();
    assert_eq!(
        paths,
        vec![
            "a.zip.eostage-conflict-20261020T000000Z",
            "a.zip.part.eostage-conflict-20261020T000000Z",
            "a.zip.part.resume.json.eostage-conflict-20261020T000000Z",
        ]
    );
    assert_eq!(listed.total_count, 3);
    assert_eq!(listed.total_size_bytes, 10 + 2 + 3);
    assert!(listed
        .files
        .iter()
        .all(|file| file.recorded == (file.path == "a.zip.eostage-conflict-20261020T000000Z")));
}

#[test]
fn recorded_conflict_names_outside_the_conflict_shape_are_rejected() {
    let root = unique_root("recorded-shape");
    let listed = list_conflict_files(
        &root,
        &[target(&root, "a.zip")],
        &["../escape.zip.part.eostage-conflict-20261020T000000Z".to_owned()],
    );
    assert!(matches!(
        listed,
        Err(ConflictCleanupError::UnsafeRecordedName(_))
    ));
    let listed = list_conflict_files(&root, &[target(&root, "a.zip")], &["a.zip".to_owned()]);
    assert!(matches!(
        listed,
        Err(ConflictCleanupError::UnsafeRecordedName(_))
    ));
}

#[test]
fn subdirectory_targets_stay_inside_the_root() {
    let root = unique_root("subdir");
    let sub = root.join("SLC");
    fs::create_dir_all(&sub).unwrap();
    let real = sub.join("b.zip.part.eostage-conflict-20261020T000000Z");
    write_file(&real, b"abc");

    let listed = list_conflict_files(
        &root,
        &[ConflictTarget {
            directory: root.join("SLC"),
            file_names: vec!["b.zip".to_owned()],
        }],
        &[],
    )
    .unwrap();
    let expected = format!(
        "SLC{}b.zip.part.eostage-conflict-20261020T000000Z",
        std::path::MAIN_SEPARATOR
    );
    assert_eq!(listed.files[0].path, expected);
    assert_eq!(listed.total_size_bytes, 3);
}

#[test]
fn delete_only_unchanged_files_and_skip_the_rest() {
    let root = unique_root("delete");
    let keep_shape = root.join("a.zip.part.eostage-conflict-20261020T000000Z");
    let changed_size = root.join("b.zip.part.eostage-conflict-20261020T000000Z");
    let changed_mtime = root.join("c.zip.part.eostage-conflict-20261020T000000Z");
    let _missing = root.join("d.zip.part.eostage-conflict-20261020T000000Z");
    write_file(&keep_shape, b"12345");
    write_file(&changed_size, b"123");
    write_file(&changed_mtime, b"12345");
    // The user confirmed the pre-change mtime; the file is then touched.
    let confirmed_mtime = mtime_ms(&changed_mtime);
    set_mtime(&changed_mtime, confirmed_mtime.saturating_add(60_000));

    let result = delete_conflict_files(
        &root,
        &[
            target(&root, "a.zip"),
            target(&root, "b.zip"),
            target(&root, "c.zip"),
            target(&root, "d.zip"),
        ],
        &[
            (
                "a.zip.part.eostage-conflict-20261020T000000Z".to_owned(),
                5,
                mtime_ms(&keep_shape),
            ),
            (
                "b.zip.part.eostage-conflict-20261020T000000Z".to_owned(),
                5,
                mtime_ms(&changed_size),
            ),
            (
                "c.zip.part.eostage-conflict-20261020T000000Z".to_owned(),
                5,
                confirmed_mtime,
            ),
            (
                "d.zip.part.eostage-conflict-20261020T000000Z".to_owned(),
                5,
                1,
            ),
        ],
    )
    .unwrap();

    assert_eq!(
        result
            .outcomes
            .iter()
            .map(|outcome| (outcome.path.as_str(), outcome.outcome.as_str()))
            .collect::<Vec<_>>(),
        vec![
            ("a.zip.part.eostage-conflict-20261020T000000Z", "deleted"),
            (
                "b.zip.part.eostage-conflict-20261020T000000Z",
                "skipped_changed"
            ),
            (
                "c.zip.part.eostage-conflict-20261020T000000Z",
                "skipped_changed"
            ),
            (
                "d.zip.part.eostage-conflict-20261020T000000Z",
                "skipped_unsafe"
            ),
        ]
    );
    assert!(!keep_shape.exists());
    assert!(changed_size.exists());
    assert!(changed_mtime.exists());
}

#[test]
fn delete_rejects_paths_outside_the_root_or_not_conflict_shaped() {
    let root = unique_root("delete-unsafe");
    let outside = root
        .parent()
        .unwrap()
        .join("outside.zip.part.eostage-conflict-20261020T000000Z");
    write_file(&outside, b"x");
    let inside_not_conflict = root.join("plain.zip");
    write_file(&inside_not_conflict, b"x");

    let refused = delete_conflict_files(
        &root,
        &[target(&root, "a.zip")],
        &[(
            "../outside.zip.part.eostage-conflict-20261020T000000Z".to_owned(),
            1,
            1,
        )],
    );
    assert!(matches!(
        refused,
        Err(ConflictCleanupError::UnsafeRequestedPath(_))
    ));

    let refused_shape = delete_conflict_files(
        &root,
        &[target(&root, "a.zip")],
        &[("plain.zip".to_owned(), 1, 1)],
    );
    assert!(matches!(
        refused_shape,
        Err(ConflictCleanupError::UnsafeRequestedPath(_))
    ));
}

#[test]
fn delete_is_idempotent() {
    let root = unique_root("idempotent");
    let file = root.join("a.zip.part.eostage-conflict-20261020T000000Z");
    write_file(&file, b"12345");
    let mtime = mtime_ms(&file);

    let first = delete_conflict_files(
        &root,
        &[target(&root, "a.zip")],
        &[(
            "a.zip.part.eostage-conflict-20261020T000000Z".to_owned(),
            5,
            mtime,
        )],
    )
    .unwrap();
    assert_eq!(first.outcomes[0].outcome, "deleted");

    let second = delete_conflict_files(
        &root,
        &[target(&root, "a.zip")],
        &[(
            "a.zip.part.eostage-conflict-20261020T000000Z".to_owned(),
            5,
            mtime,
        )],
    )
    .unwrap();
    assert_eq!(second.outcomes[0].outcome, "skipped_unsafe");
}

#[test]
fn other_tasks_files_are_untouched() {
    let root = unique_root("other-tasks");
    let mine = root.join("mine");
    let theirs = root.join("theirs");
    fs::create_dir_all(&mine).unwrap();
    fs::create_dir_all(&theirs).unwrap();
    let my_file = mine.join("a.zip.part.eostage-conflict-20261020T000000Z");
    let their_file = theirs.join("a.zip.part.eostage-conflict-20261020T000000Z");
    write_file(&my_file, b"12345");
    write_file(&their_file, b"12345");

    // Only files under the task's own target directories are visible; the
    // other directory's conflict file is neither listed nor deleted.
    let listed = list_conflict_files(&root, &[target(&mine, "a.zip")], &[]).unwrap();
    assert_eq!(listed.total_count, 1);

    let relative = format!(
        "mine{}a.zip.part.eostage-conflict-20261020T000000Z",
        std::path::MAIN_SEPARATOR
    );
    let result = delete_conflict_files(
        &root,
        &[target(&mine, "a.zip")],
        &[(relative, 5, mtime_ms(&my_file))],
    )
    .unwrap();
    assert_eq!(result.outcomes[0].outcome, "deleted");
    assert!(!my_file.exists());
    assert!(their_file.exists());
}

#[test]
fn a_directory_named_like_a_conflict_file_is_never_listed_or_deleted() {
    let root = unique_root("unsafe-entry");
    let directory = root.join("a.zip.part.eostage-conflict-20261020T000000Z");
    fs::create_dir_all(&directory).unwrap();
    fs::write(directory.join("inner"), b"x").unwrap();

    let listed = list_conflict_files(&root, &[target(&root, "a.zip")], &[]).unwrap();
    assert_eq!(listed.total_count, 0);

    let result = delete_conflict_files(
        &root,
        &[target(&root, "a.zip")],
        &[(
            "a.zip.part.eostage-conflict-20261020T000000Z".to_owned(),
            1,
            1,
        )],
    )
    .unwrap();
    assert_eq!(result.outcomes[0].outcome, "skipped_unsafe");
    assert!(directory.is_dir());
    assert!(directory.join("inner").exists());
}

#[test]
fn a_freshly_touched_file_is_reported_changed_not_deleted() {
    let root = unique_root("mtime-drift");
    let file = root.join("a.zip.part.eostage-conflict-20261020T000000Z");
    write_file(&file, b"12345");
    // The listed mtime is one second behind the file's real mtime.
    let stale = mtime_ms(&file).saturating_sub(1_000);
    let result = delete_conflict_files(
        &root,
        &[target(&root, "a.zip")],
        &[(
            "a.zip.part.eostage-conflict-20261020T000000Z".to_owned(),
            5,
            stale,
        )],
    )
    .unwrap();
    assert_eq!(result.outcomes[0].outcome, "skipped_changed");
    assert!(file.exists());
}

#[test]
fn zero_targets_lists_nothing() {
    let root = unique_root("empty");
    let listed = list_conflict_files(&root, &[], &[]).unwrap();
    assert_eq!(listed.total_count, 0);
    assert_eq!(listed.total_size_bytes, 0);
    assert!(listed.files.is_empty());
}

#[cfg(unix)]
#[test]
fn symlinks_are_rejected() {
    let root = unique_root("symlink");
    let destination = root.join("linked");
    fs::write(root.join("linked-target"), b"target").unwrap();
    std::os::unix::fs::symlink(root.join("linked-target"), &destination).unwrap();
    let listed = list_conflict_files(&root, &[target(&destination, "a.zip")], &[]);
    assert!(matches!(listed, Err(ConflictCleanupError::UnsafeEntry(_))));
    let real_dir = root.join("real-dir");
    fs::create_dir_all(&real_dir).unwrap();
    let inside = real_dir.join("a.zip.part.eostage-conflict-20261020T000000Z");
    std::os::unix::fs::symlink(root.join("linked-target"), &inside).unwrap();
    let listed = list_conflict_files(&root, &[target(&real_dir, "a.zip")], &[]);
    assert_eq!(listed.unwrap().total_count, 0);
}

/// A conflict-shaped file in a directory that is not one of the task's own
/// target directories is refused, even under the same root.
#[test]
fn delete_refuses_files_outside_the_task_target_directories() {
    let root = unique_root("delete-foreign-dir");
    let foreign = root.join("other-task");
    fs::create_dir_all(&foreign).unwrap();
    let file = foreign.join("a.zip.part.eostage-conflict-20261020T000000Z");
    let size = write_file(&file, b"foreign");
    let result = delete_conflict_files(
        &root,
        &[target(&root, "a.zip")],
        &[(
            r"other-task\a.zip.part.eostage-conflict-20261020T000000Z".to_owned(),
            size,
            mtime_ms(&file),
        )],
    );
    assert!(matches!(
        result,
        Err(ConflictCleanupError::UnsafeRequestedPath(_))
    ));
    assert!(
        file.exists(),
        "a file outside the task targets is never deleted"
    );
    fs::remove_dir_all(root).unwrap();
}
