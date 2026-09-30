/*
 * Licensed to the Apache Software Foundation (ASF) under one
 * or more contributor license agreements.  See the NOTICE file
 * distributed with this work for additional information
 * regarding copyright ownership.  The ASF licenses this file
 * to you under the Apache License, Version 2.0 (the
 * "License"); you may not use this file except in compliance
 * with the License.  You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing,
 * software distributed under the License is distributed on an
 * "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
 * KIND, either express or implied.  See the License for the
 * specific language governing permissions and limitations
 * under the License.
 */

use super::snapshot::Snapshot;
use maka_event_log::root::{FileLease, private_directory};
#[cfg(any(unix, test))]
use std::fs;
#[cfg(unix)]
use std::fs::File;
use std::{
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

pub(super) const MAX_BYTES: u64 = 40 * 1024 * 1024;

pub(super) fn fits(bytes: usize) -> bool {
    bytes as u64 <= MAX_BYTES
}

pub(super) fn encode(value: &impl serde::Serialize) -> io::Result<Vec<u8>> {
    let bytes = serde_json::to_vec(value)?;
    if !fits(bytes.len()) {
        return Err(io::Error::other("TUI checkpoint exceeds local capacity"));
    }
    Ok(bytes)
}

pub struct Store {
    pub root: String,
    path: PathBuf,
    lease: FileLease,
    /// Reclamation watermark from the last committed checkpoint. Pending new
    /// files are not in this set and cannot be removed by an older save.
    clipboard_files: std::collections::HashSet<PathBuf>,
}
impl Store {
    pub fn open(base: &Path, root: &str, profile: &str) -> io::Result<(Self, Option<Snapshot>)> {
        if !base.is_absolute()
            || root.len() != 64
            || !root.bytes().all(|c| c.is_ascii_hexdigit())
            || profile.is_empty()
            || profile.len() > 64
            || !profile
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_'))
        {
            return Err(io::Error::other(
                "Invalid TUI state directory, Root or profile",
            ));
        }
        private_directory(base)?;
        let directory = base.join(root).join(profile);
        private_directory(&directory)?;
        let lease = FileLease::acquire(&directory.join("writer.lock"))?;
        let path = directory.join("state.json");
        let bytes = read(&path)?.unwrap_or_default();
        let saved = if bytes.is_empty() {
            if path.exists() {
                return Err(io::Error::other(
                    "Empty TUI checkpoint; original file preserved",
                ));
            }
            None
        } else {
            let saved: Snapshot = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
            saved.validate(root).map_err(io::Error::other)?;
            Some(saved)
        };
        let clipboard = directory.join("clipboard");
        private_directory(&clipboard)?;
        let clipboard_files = saved
            .as_ref()
            .map(|saved| saved.clipboard_files(&clipboard))
            .unwrap_or_default();
        // The profile lease makes startup the safe place to reclaim files from
        // interrupted imports that never reached a committed checkpoint.
        for entry in std::fs::read_dir(&clipboard)? {
            let entry = entry?;
            if entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.starts_with("clipboard-") && name.ends_with(".png"))
                && entry.file_type()?.is_file()
                && !clipboard_files.contains(&entry.path())
            {
                let _ = std::fs::remove_file(entry.path());
            }
        }
        Ok((
            Self {
                root: root.into(),
                path,
                lease,
                clipboard_files,
            },
            saved,
        ))
    }
    pub fn clipboard_directory(&self) -> PathBuf {
        self.path
            .parent()
            .expect("owned state directory")
            .join("clipboard")
    }
    pub fn save(&mut self, snapshot: Snapshot) -> io::Result<()> {
        snapshot.validate(&self.root).map_err(io::Error::other)?;
        let bytes = encode(&snapshot)?;
        self.lease.validate()?;
        // Check the target before replacing it; never follow a link or a device.
        let current = read(&self.path)?;
        if current.as_deref() == Some(bytes.as_slice()) {
            return Ok(());
        }
        let directory = self.path.parent().expect("owned state directory");
        let mut temp = tempfile::NamedTempFile::new_in(directory)?;
        temp.write_all(&bytes)?;
        temp.as_file().sync_all()?;
        self.lease.validate()?;
        replace(temp, &self.path)?;
        let retained = snapshot.clipboard_files(&self.clipboard_directory());
        for path in self.clipboard_files.difference(&retained) {
            let _ = std::fs::remove_file(path);
        }
        self.clipboard_files = retained;
        Ok(())
    }
}

fn read(path: &Path) -> io::Result<Option<Vec<u8>>> {
    match path.symlink_metadata() {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
        Ok(meta) if !meta.is_file() || meta.len() > MAX_BYTES => {
            return Err(io::Error::other("Invalid TUI checkpoint file"));
        }
        _ => {}
    }
    #[cfg(unix)]
    let file = {
        use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
        let file = fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(path)?;
        let meta = file.metadata()?;
        if !meta.is_file()
            || meta.nlink() != 1
            || meta.uid() != unsafe { libc::geteuid() }
            || meta.mode() & 0o077 != 0
        {
            return Err(io::Error::other(
                "TUI checkpoint is not a private regular file",
            ));
        }
        file
    };
    #[cfg(windows)]
    let file = {
        let file = maka_event_log::root::windows::open_nofollow(path, false)?;
        maka_event_log::root::windows::validate_private(&file)?;
        if !file.metadata()?.is_file()
            || maka_event_log::root::windows::file_identity(&file)?.links != 1
        {
            return Err(io::Error::other(
                "TUI checkpoint is not a private regular file",
            ));
        }
        file
    };
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(io::Error::other("TUI checkpoint exceeds local capacity"));
    }
    Ok(Some(bytes))
}

#[cfg(unix)]
fn replace(temp: tempfile::NamedTempFile, target: &Path) -> io::Result<()> {
    temp.persist(target).map_err(|e| e.error)?;
    File::open(target.parent().expect("state directory"))?.sync_all()
}

#[cfg(windows)]
fn replace(temp: tempfile::NamedTempFile, target: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };
    let source: Vec<u16> = temp
        .path()
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    let target: Vec<u16> = target.as_os_str().encode_wide().chain(Some(0)).collect();
    if unsafe {
        MoveFileExW(
            source.as_ptr(),
            target.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        app::{Action, App},
        i18n::{I18n, Locale, LocalePreference},
        navigation::Route,
    };
    const ROOT: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    #[test]
    fn clipboard_drafts_survive_restart_and_cleanup_follows_committed_reachability() {
        let directory = tempfile::tempdir().unwrap();
        let (mut store, _) = Store::open(directory.path(), ROOT, "default").unwrap();
        let mut app = App::new(
            "/unused".into(),
            I18n::new(LocalePreference::Auto, Locale::En),
        );
        app.apply(Action::Visit(Route::Session("a".into())));
        let image = store.clipboard_directory().join("clipboard-kept.png");
        let external = directory.path().join("user-file.png");
        for path in [&image, &external] {
            fs::write(path, b"image bytes").unwrap();
            app.attachments.saved.entry("a".into()).or_default().push(
                crate::pages::attachments::Saved {
                    id: uuid::Uuid::new_v4().to_string(),
                    path: path.clone(),
                    manifest: None,
                    attachment: None,
                },
            );
        }
        store.save(Snapshot::capture(&app, ROOT)).unwrap();
        let pending = store.clipboard_directory().join("clipboard-pending.png");
        fs::write(&pending, b"an import not adopted by the UI yet").unwrap();
        app.drafts.get_mut("a").unwrap().insert("new text");
        store.save(Snapshot::capture(&app, ROOT)).unwrap();
        assert!(
            pending.exists(),
            "an earlier checkpoint cannot collect an in-flight image"
        );
        drop(store);
        let (mut store, saved) = Store::open(directory.path(), ROOT, "default").unwrap();
        assert!(!pending.exists(), "startup collects abandoned imports");
        assert!(image.exists());
        let mut restored = App::new(
            "/unused".into(),
            I18n::new(LocalePreference::Auto, Locale::En),
        );
        saved.unwrap().restore(&mut restored, false).unwrap();
        assert_eq!(restored.attachments.saved["a"][0].path, image);
        restored.attachments.saved.clear();
        let mut invalid = Snapshot::capture(&restored, ROOT);
        invalid.root = "wrong-root".into();
        assert!(store.save(invalid).is_err());
        assert!(
            image.exists(),
            "a rejected checkpoint must not delete its predecessor's files"
        );
        store.save(Snapshot::capture(&restored, ROOT)).unwrap();
        assert!(!image.exists());
        assert!(
            external.exists(),
            "user-owned source files are never reclaimed"
        );
    }

    #[test]
    fn private_atomic_checkpoint_locks_profiles_and_preserves_invalid_files_and_links() {
        let directory = tempfile::tempdir().unwrap();
        let (mut store, saved) = Store::open(directory.path(), ROOT, "default").unwrap();
        assert!(saved.is_none());
        assert!(Store::open(directory.path(), ROOT, "default").is_err());
        assert!(Store::open(directory.path(), ROOT, "second").is_ok());
        let mut app = App::new(
            "/unused".into(),
            I18n::new(LocalePreference::Auto, Locale::En),
        );
        app.apply(Action::Visit(Route::Session("a".into())));
        app.drafts.get_mut("a").unwrap().insert("中文🦀");
        store.save(Snapshot::capture(&app, ROOT)).unwrap();
        let path = store.path.clone();
        let before = fs::read(&path).unwrap();
        drop(store);
        let (mut store, saved) = Store::open(directory.path(), ROOT, "default").unwrap();
        let mut restored = App::new(
            "/unused".into(),
            I18n::new(LocalePreference::Auto, Locale::En),
        );
        saved.unwrap().restore(&mut restored, false).unwrap();
        assert_eq!(restored.drafts["a"].text(), "中文🦀");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
            fs::remove_file(&path).unwrap();
            std::os::unix::fs::symlink(directory.path().join("untouched"), &path).unwrap();
            assert!(
                store
                    .save(Snapshot::capture(
                        &App::new(
                            "/unused".into(),
                            I18n::new(LocalePreference::Auto, Locale::En)
                        ),
                        ROOT
                    ))
                    .is_err()
            );
            assert!(!directory.path().join("untouched").exists());
            fs::remove_file(&path).unwrap();
        }
        drop(store);
        // Malformed and future-version files are not replaced by defaults.
        fs::write(&path, b"{broken").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        }
        assert!(Store::open(directory.path(), ROOT, "default").is_err());
        assert_eq!(fs::read(&path).unwrap(), b"{broken");
        for (pointer, value) in [
            ("/version", serde_json::json!(20)),
            ("/version", serde_json::json!(u32::MAX)),
            (
                "/navigation/entries/0",
                serde_json::json!({"page":"workspace"}),
            ),
            (
                "/navigation/entries/0/settings",
                serde_json::json!({"kind":"builtin","value":"host"}),
            ),
        ] {
            let mut invalid: serde_json::Value = serde_json::from_slice(&before).unwrap();
            *invalid.pointer_mut(pointer).unwrap() = value;
            let bytes = serde_json::to_vec_pretty(&invalid).unwrap();
            fs::write(&path, &bytes).unwrap();
            let failure = match Store::open(directory.path(), ROOT, "default") {
                Ok(_) => panic!("invalid checkpoint opened a writer: {pointer}"),
                Err(error) => error,
            };
            assert!(!failure.to_string().is_empty());
            assert_eq!(fs::read(&path).unwrap(), bytes, "{pointer}");
        }
        fs::write(&path, before).unwrap();
        assert!(Store::open(directory.path(), ROOT, "default").is_ok());
        assert!(Store::open(directory.path(), ROOT, "../escape").is_err());
    }
}
