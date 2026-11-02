use super::CharacterLibrary;
use crate::{
    Result,
    assets::private_directory,
    project::persistence::{atomic_write, read_limited},
};
use std::{
    fs::{self, OpenOptions},
    os::{fd::AsRawFd, unix::fs::OpenOptionsExt},
    path::{Path, PathBuf},
};

/// All mutations use a private flock plus an exact baseline comparison, then atomic replacement.
pub struct CharacterStore {
    pub library: CharacterLibrary,
    pub error: Option<String>,
    path: PathBuf,
    baseline: Option<String>,
}
struct FileLock(fs::File);
impl Drop for FileLock {
    fn drop(&mut self) {
        // SAFETY: this File owns the successfully locked fd. Explicit unlock also
        // releases fork-inherited copies before those children reach exec/exit.
        unsafe {
            libc::flock(self.0.as_raw_fd(), libc::LOCK_UN);
        }
    }
}
fn read(path: &Path) -> Result<Option<String>> {
    match fs::symlink_metadata(path) {
        Ok(_) => read_limited(path, 4 * 1024 * 1024).map(Some),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}
impl CharacterStore {
    pub fn load(config: &Path) -> Self {
        let path = config.join("characters.json");
        let mut store = Self {
            library: Default::default(),
            error: None,
            path,
            baseline: None,
        };
        let result = (|| {
            store.baseline = read(&store.path)?;
            if let Some(text) = &store.baseline {
                let library: CharacterLibrary =
                    serde_json::from_str(text).map_err(|e| e.to_string())?;
                library.validate()?;
                store.library = library;
            }
            Ok::<_, String>(())
        })();
        if let Err(error) = result {
            store.error = Some(format!("characters-corrupt: {error}"));
        }
        store
    }
    pub fn save(&mut self, library: CharacterLibrary) -> Result<()> {
        if self.error.is_some() {
            return Err("characters-protected".into());
        }
        library.validate()?;
        let bytes = serde_json::to_vec_pretty(&library).map_err(|e| e.to_string())?;
        if bytes.len() > 4 * 1024 * 1024 {
            return Err("file-too-large".into());
        }
        let parent = self.path.parent().ok_or("invalid-path")?;
        private_directory(parent)?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(parent.join("characters.lock"))
            .map_err(|e| e.to_string())?;
        // SAFETY: lock owns an open fd; FileLock releases the acquired lock on all exits.
        if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err("characters-busy".into());
        }
        let _lock = FileLock(lock);
        if read(&self.path)? != self.baseline {
            return Err("characters-conflict".into());
        }
        atomic_write(&self.path, &bytes)?;
        self.baseline = Some(String::from_utf8(bytes).map_err(|e| e.to_string())?);
        self.library = library;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unlocking_does_not_wait_for_inherited_file_descriptors() {
        let path = std::env::temp_dir().join(format!(
            "vmm-character-lock-{}",
            crate::project::Id::new_v4()
        ));
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
            .unwrap();
        // SAFETY: file is an open fd owned by this test; nonblocking flock cannot hang.
        assert_eq!(
            unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
            0
        );
        // Like a descriptor inherited by fork, try_clone shares the open-file description.
        let inherited = file.try_clone().unwrap();
        drop(FileLock(file));
        let other = fs::File::open(&path).unwrap();
        // SAFETY: other is an independent, valid open fd; this only tests lock availability.
        assert_eq!(
            unsafe { libc::flock(other.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
            0
        );
        drop(FileLock(other));
        drop(inherited);
        fs::remove_file(path).unwrap();
    }
}
