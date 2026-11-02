use super::Project;
use crate::Result;
use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    os::unix::fs::OpenOptionsExt,
    path::Path,
};
use uuid::Uuid;

pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let temporary = parent.join(format!(".voxmoviemaker-{}.tmp", Uuid::new_v4()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)
            .map_err(|e| e.to_string())?;
        file.write_all(bytes)
            .and_then(|()| file.sync_all())
            .map_err(|e| e.to_string())?;
        fs::rename(&temporary, path).map_err(|e| e.to_string())?;
        if let Ok(dir) = fs::File::open(parent) {
            let _ = dir.sync_all();
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}
pub fn save(path: &Path, project: &Project) -> Result<()> {
    project.validate()?;
    let bytes = serde_json::to_vec_pretty(project).map_err(|e| e.to_string())?;
    if bytes.len() > 16 * 1024 * 1024 {
        return Err("project-too-large".into());
    }
    atomic_write(path, &bytes)
}
pub fn load(path: &Path) -> Result<Project> {
    let input = read_limited(path, 16 * 1024 * 1024).map_err(|e| {
        if e == "file-too-large" {
            "project-too-large".into()
        } else {
            e
        }
    })?;
    let mut project: Project = serde_json::from_str(&input).map_err(|e| e.to_string())?;
    // Preserve the appearance of old projects: missing integrated subtitles stay disabled.
    if matches!(project.schema_version, 1..=3) {
        project.schema_version = super::SCHEMA_VERSION;
    }
    let parent = path
        .canonicalize()
        .map_err(|e| e.to_string())?
        .parent()
        .ok_or("invalid-path")?
        .to_path_buf();
    for asset in &mut project.assets {
        if let super::AssetLocation::External(p) = &mut asset.location
            && p.is_relative()
        {
            *p = parent.join(&*p);
        }
    }
    project.validate()?;
    Ok(project)
}
pub fn read_limited(path: &Path, limit: u64) -> Result<String> {
    let mut input = String::new();
    fs::File::open(path)
        .map_err(|e| e.to_string())?
        .take(limit + 1)
        .read_to_string(&mut input)
        .map_err(|e| e.to_string())?;
    if input.len() as u64 > limit {
        return Err("file-too-large".into());
    }
    Ok(input)
}
