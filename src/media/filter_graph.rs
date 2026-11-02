//! RAII storage for filter graphs which cannot fit in a Linux argv entry.
use crate::{
    Result,
    assets::{private_directory, resolved_path},
};
use std::{
    fs,
    io::Write,
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
};

pub struct FilterGraphFile {
    pub path: PathBuf,
}
impl FilterGraphFile {
    pub fn create(graph: &str, scratch: &Path, data: &Path) -> Result<Self> {
        let cache = resolved_path(scratch)?;
        let data = resolved_path(data)?;
        if cache.starts_with(&data) || data.starts_with(&cache) {
            return Err("data-cache-overlap".into());
        }
        private_directory(scratch)?;
        let path = scratch.join(format!("vmm-filter-{}.txt", uuid::Uuid::new_v4()));
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&path)
            .map_err(|e| e.to_string())?;
        let guard = Self { path };
        file.write_all(graph.as_bytes())
            .map_err(|e| e.to_string())?;
        Ok(guard)
    }
}
impl Drop for FilterGraphFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}
