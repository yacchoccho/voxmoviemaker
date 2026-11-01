//! Explicit project-with-assets export. Never bundles fonts or external software.
use super::{persistence, *};
use crate::{
    Result,
    assets::{Storage, ensure_outside_git, resolved_path},
};
use std::{
    collections::HashSet,
    ffi::CString,
    fs,
    io::{Read, Write},
    os::unix::{
        ffi::OsStrExt,
        fs::{DirBuilderExt, OpenOptionsExt},
    },
    path::Path,
};

pub fn export(
    project: &Project,
    storage: &Storage,
    target: &Path,
    cancelled: impl Fn() -> bool,
    progress: impl Fn(f32),
) -> Result<()> {
    project.validate()?;
    let target = resolved_path(target)?;
    if target.exists() {
        return Err("output-exists".into());
    }
    ensure_outside_git(&target).map_err(|_| "package-in-git")?;
    let parent = target.parent().ok_or("invalid-path")?;
    let used: HashSet<_> = project
        .tracks
        .iter()
        .flat_map(|t| &t.clips)
        .filter_map(|c| c.source.asset_id())
        .collect();
    let mut portable = project.clone();
    portable.assets.retain(|a| used.contains(&a.id));
    let mut total = 0u64;
    for asset in &portable.assets {
        let path = storage.resolve(project.project_uuid, asset);
        let metadata = fs::metadata(&path).map_err(|e| format!("missing-asset: {e}"))?;
        if !metadata.is_file() {
            return Err("missing-asset".into());
        }
        let mut header = [0; 16];
        let count = fs::File::open(&path)
            .and_then(|mut f| f.read(&mut header))
            .map_err(|e| e.to_string())?;
        let head = &header[..count];
        if [
            b"\x7fELF".as_slice(),
            b"MZ",
            b"#!",
            b"!<arch>",
            b"PK\x03\x04",
            b"OTTO",
            b"ttcf",
            b"\x00\x01\x00\x00",
        ]
        .iter()
        .any(|magic| head.starts_with(magic))
        {
            return Err("unsupported-media".into());
        }
        total = total.checked_add(metadata.len()).ok_or("file-too-large")?;
    }
    let stage = parent.join(format!(".voxmoviemaker-package-{}", Id::new_v4()));
    fs::DirBuilder::new()
        .mode(0o700)
        .create(&stage)
        .map_err(|e| e.to_string())?;
    let result = (|| {
        fs::DirBuilder::new()
            .mode(0o700)
            .create(stage.join("media"))
            .map_err(|e| e.to_string())?;
        let mut copied = 0u64;
        for asset in &mut portable.assets {
            if cancelled() {
                return Err("cancelled".into());
            }
            let source = storage.resolve(project.project_uuid, asset);
            let before = fs::metadata(&source).map_err(|e| e.to_string())?;
            let extension = source
                .extension()
                .and_then(|s| s.to_str())
                .filter(|s| s.len() <= 10 && s.chars().all(|ch| ch.is_ascii_alphanumeric()))
                .unwrap_or("bin");
            let relative =
                std::path::PathBuf::from("media").join(format!("{}.{extension}", asset.id));
            let mut input = fs::File::open(&source).map_err(|e| e.to_string())?;
            let mut output = fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .mode(0o600)
                .open(stage.join(&relative))
                .map_err(|e| e.to_string())?;
            let mut buffer = [0; 65536];
            loop {
                if cancelled() {
                    return Err("cancelled".into());
                }
                let count = input.read(&mut buffer).map_err(|e| e.to_string())?;
                if count == 0 {
                    break;
                }
                output
                    .write_all(&buffer[..count])
                    .map_err(|e| e.to_string())?;
                copied = copied.saturating_add(count as u64);
                progress((copied as f64 / total.max(1) as f64).min(0.99) as f32);
            }
            output.sync_all().map_err(|e| e.to_string())?;
            let after = fs::metadata(&source).map_err(|e| e.to_string())?;
            if before.len() != after.len() || before.modified().ok() != after.modified().ok() {
                return Err("source-changed".into());
            }
            // A copied VOICEVOX WAV becomes a portable external asset, while the
            // ClipSource retains its text/settings for regeneration into XDG data.
            asset.location = AssetLocation::External(relative);
        }
        persistence::save(&stage.join("project.vmm.json"), &portable)?;
        fs::write(stage.join("README.txt"),"VoxMovieMaker portable project\nOpen project.vmm.json. The media directory contains explicitly copied user assets, including speech. Do not publish it without reviewing the contents and applicable voice/media terms. Fonts, FFmpeg and VOICEVOX are not included. Install matching system fonts separately.\n").map_err(|e|e.to_string())?;
        if cancelled() {
            return Err("cancelled".into());
        }
        ensure_outside_git(&target).map_err(|_| "package-in-git")?;
        let from = CString::new(stage.as_os_str().as_bytes()).map_err(|_| "invalid-path")?;
        let to = CString::new(target.as_os_str().as_bytes()).map_err(|_| "invalid-path")?;
        // SAFETY: NUL-terminated paths. Linux atomic publication never replaces an
        // existing destination, including a directory created after our initial check.
        if unsafe {
            libc::renameat2(
                libc::AT_FDCWD,
                from.as_ptr(),
                libc::AT_FDCWD,
                to.as_ptr(),
                libc::RENAME_NOREPLACE,
            )
        } != 0
        {
            return Err(std::io::Error::last_os_error().to_string());
        }
        if let Ok(directory) = fs::File::open(parent) {
            let _ = directory.sync_all();
        }
        progress(1.0);
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&stage);
    }
    result
}
