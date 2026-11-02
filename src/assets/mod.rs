use crate::{
    Result,
    project::{AssetLocation, Id, MediaAsset},
};
use std::{
    env, fs,
    io::Write,
    os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    path::{Path, PathBuf},
};

#[derive(Clone, Debug)]
pub struct Storage {
    pub data: PathBuf,
    pub cache: PathBuf,
    pub config: PathBuf,
}
pub fn xdg_path(value: Option<PathBuf>, home: &Path, fallback: &str) -> PathBuf {
    value
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home.join(fallback))
        .join("voxmoviemaker")
}
impl Storage {
    pub fn discover() -> Result<Self> {
        let home = env::var_os("HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .ok_or("missing-home")?;
        Ok(Self {
            data: xdg_path(
                env::var_os("XDG_DATA_HOME").map(PathBuf::from),
                &home,
                ".local/share",
            ),
            cache: xdg_path(
                env::var_os("XDG_CACHE_HOME").map(PathBuf::from),
                &home,
                ".cache",
            ),
            config: xdg_path(
                env::var_os("XDG_CONFIG_HOME").map(PathBuf::from),
                &home,
                ".config",
            ),
        })
    }
    pub fn validate_data_root(&self, path: &Path) -> Result<()> {
        require_utf8_path(path)?;
        ensure_outside_git(path)?;
        let path = resolved_path(path)?;
        let cache = resolved_path(&self.cache)?;
        if path.starts_with(&cache) || cache.starts_with(&path) {
            return Err("data-cache-overlap".into());
        }
        Ok(())
    }
    pub fn voice_path(&self, project: Id, voice: Id) -> PathBuf {
        self.data
            .join("projects")
            .join(project.to_string())
            .join("voice")
            .join(format!("{voice}.wav"))
    }
    pub fn resolve(&self, project: Id, asset: &MediaAsset) -> PathBuf {
        match &asset.location {
            AssetLocation::External(path) => path.clone(),
            AssetLocation::Voice(id) => self.voice_path(project, *id),
        }
    }
    pub fn save_voice(&self, project: Id, voice: Id, wav: &[u8]) -> Result<PathBuf> {
        self.validate_data_root(&self.data)?;
        let path = self.voice_path(project, voice);
        let parent = path.parent().ok_or("invalid-path")?;
        ensure_outside_git(parent)?;
        private_directory(parent)?;
        ensure_outside_git(parent)?;
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&path)
            .map_err(|e| e.to_string())?;
        if let Err(e) = file.write_all(wav).and_then(|()| file.sync_all()) {
            // Only this newly-created incomplete file is removed. Existing WAVs are immutable.
            let _ = fs::remove_file(&path);
            return Err(e.to_string());
        }
        Ok(path)
    }
}
pub fn private_directory(path: &Path) -> Result<()> {
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)
        .map_err(|e| e.to_string())
}
pub fn resolved_path(path: &Path) -> Result<PathBuf> {
    if !path.is_absolute() {
        return Err("absolute-path-required".into());
    }
    let mut existing = path;
    let mut tail = Vec::new();
    while !existing.exists() {
        tail.push(existing.file_name().ok_or("invalid-path")?.to_os_string());
        existing = existing.parent().ok_or("invalid-path")?;
    }
    let mut result = existing.canonicalize().map_err(|e| e.to_string())?;
    for part in tail.iter().rev() {
        result.push(part);
    }
    Ok(result)
}
pub fn ensure_outside_git(path: &Path) -> Result<()> {
    let resolved = resolved_path(path)?;
    for ancestor in resolved.ancestors() {
        if ancestor.join(".git").exists()
            || (ancestor.join("HEAD").is_file() && ancestor.join("objects").is_dir())
        {
            return Err("voice-storage-in-git".into());
        }
    }
    Ok(())
}

pub fn require_utf8_path(path: &Path) -> Result<()> {
    if path.to_str().is_none() {
        return Err("non-utf8-path".into());
    }
    Ok(())
}
