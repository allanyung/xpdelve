// Adapted from Sofka's src/atomicfile.rs.
// Copyright 2026 Nikola Milojević. Licensed under Apache-2.0.
// Translated and modified for xpdelve in 2026. See NOTICE.
//! Crash-safe replacement of the small files xpdelve rewrites.
//!
//! The bytes go to a uniquely named sibling temp file that is synced and then
//! renamed over the target, so a reader sees either the whole previous file or
//! the whole new one. The temp name is claimed with `create_new`, so a writer
//! can never truncate a file another writer is still filling.

use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{self, ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);
const TEMP_ATTEMPTS: u32 = 16;

/// Replace `path` with `contents`, creating the parent directory if needed.
pub fn write(path: &Path, contents: &str) -> io::Result<()> {
    write_with(path, contents, temp_path)
}

fn write_with(
    path: &Path,
    contents: &str,
    mut candidate: impl FnMut(&Path) -> PathBuf,
) -> io::Result<()> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)?;
    }
    let (mut file, temp) = create_temp(path, &mut candidate)?;
    let result = file
        .write_all(contents.as_bytes())
        .and_then(|()| file.sync_all())
        .and_then(|()| {
            drop(file);
            fs::rename(&temp, path)
        });
    if result.is_err() {
        // Only ever this call's own file: `create_new` refused existing names.
        let _ = fs::remove_file(&temp);
    }
    result
}

fn create_temp(
    path: &Path,
    candidate: &mut impl FnMut(&Path) -> PathBuf,
) -> io::Result<(File, PathBuf)> {
    let mut taken = None;
    for _ in 0..TEMP_ATTEMPTS {
        let temp = candidate(path);
        match File::options().write(true).create_new(true).open(&temp) {
            Ok(file) => return Ok((file, temp)),
            Err(error) if error.kind() == ErrorKind::AlreadyExists => taken = Some(error),
            Err(error) => return Err(error),
        }
    }
    Err(taken
        .unwrap_or_else(|| io::Error::new(ErrorKind::AlreadyExists, "no free temporary file name")))
}

// Renames are only atomic within one filesystem, so the temp file is a sibling.
fn temp_path(path: &Path) -> PathBuf {
    let counter = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.subsec_nanos());
    let mut name = OsString::from(path.as_os_str());
    name.push(format!(".tmp{}.{counter:x}.{nanos:x}", std::process::id()));
    PathBuf::from(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_missing_parents_and_replaces_contents() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("a/b/state.toml");
        write(&path, "one").unwrap();
        write(&path, "two").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "two");
        let leftovers = fs::read_dir(path.parent().unwrap()).unwrap().count();
        assert_eq!(leftovers, 1);
    }

    #[test]
    fn a_taken_temp_name_is_skipped_not_truncated() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("state.toml");
        let taken = directory.path().join("taken");
        fs::write(&taken, "other writer").unwrap();
        let mut names = vec![taken.clone(), directory.path().join("free")].into_iter();
        write_with(&path, "mine", |_| names.next().unwrap()).unwrap();
        assert_eq!(fs::read_to_string(&taken).unwrap(), "other writer");
        assert_eq!(fs::read_to_string(&path).unwrap(), "mine");
    }

    #[test]
    fn a_failed_rename_removes_the_temp_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("target");
        fs::create_dir(&path).unwrap();
        fs::write(path.join("child"), "x").unwrap();
        assert!(write(&path, "data").is_err());
        let names: Vec<_> = fs::read_dir(directory.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(names.len(), 1);
    }
}
