//! One deterministic image sequence for preview, proxy and export. Only unique
//! visibility states become PNGs; long idle stretches occupy one manifest entry.
use super::{SpriteRender, psd};
use crate::{
    Result,
    assets::{private_directory, resolved_path},
    project::{FrameRate, Id},
};
use std::{
    collections::{BTreeSet, HashMap},
    fs,
    io::Write,
    os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    path::{Path, PathBuf},
};

pub struct SpriteFiles {
    pub input: PathBuf,
    pub animated: bool,
    directory: PathBuf,
}
impl Drop for SpriteFiles {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}
pub struct SpriteInput<'a> {
    pub sprite: &'a SpriteRender,
    pub fps: FrameRate,
    pub start: u64,
    pub offset: u64,
    pub duration: u64,
    pub preview: Option<u64>,
    pub size: [u32; 2],
    pub scratch: &'a Path,
    pub data: &'a Path,
}
pub fn prepare(input: SpriteInput<'_>, cancelled: &dyn Fn() -> bool) -> Result<SpriteFiles> {
    let SpriteInput {
        sprite,
        fps,
        start,
        offset,
        duration,
        preview,
        size,
        scratch,
        data,
    } = input;
    let cache = resolved_path(scratch)?;
    let data = resolved_path(data)?;
    if cache.starts_with(&data) || data.starts_with(&cache) {
        return Err("data-cache-overlap".into());
    }
    if cancelled() {
        return Err("cancelled".into());
    }
    if sprite
        .source_revision
        .is_some_and(|revision| Some(revision) != psd::revision(&sprite.settings.psd))
    {
        return Err("sprite-psd-changed".into());
    }
    let resolved = super::speech::resolve(sprite, fps, &scratch.join("speech"), cancelled)?;
    let sprite = &resolved;
    let doc = psd::load(&sprite.settings.psd)?;
    doc.validate_settings(&sprite.settings)?;
    private_directory(scratch)?;
    let directory = scratch.join(format!("vmm-sprite-{}", Id::new_v4()));
    fs::DirBuilder::new()
        .mode(0o700)
        .create(&directory)
        .map_err(|e| e.to_string())?;
    let animated = preview.is_none();
    let mut files = SpriteFiles {
        input: directory.join(if animated {
            "sequence.ffconcat"
        } else {
            "frame.png"
        }),
        animated,
        directory,
    };
    let mut seen: HashMap<BTreeSet<String>, String> = HashMap::new();
    let mut bytes = 0u64;
    let mut image_for = |frame: u64| -> Result<String> {
        if cancelled() {
            return Err("cancelled".into());
        }
        let state = doc.visibility(sprite, offset + frame, start + frame, fps)?;
        if let Some(name) = seen.get(&state) {
            return Ok(name.clone());
        }
        if seen.len() >= 1024 {
            return Err("sprite-sequence-too-large".into());
        }
        let image = doc.composite_cancellable(&state, cancelled)?;
        let image = image::imageops::resize(
            &image,
            size[0],
            size[1],
            image::imageops::FilterType::Triangle,
        );
        let name = if animated {
            format!("frame-{}.png", seen.len())
        } else {
            "frame.png".into()
        };
        let path = files.directory.join(&name);
        image.save(&path).map_err(|e| e.to_string())?;
        bytes += fs::metadata(&path).map_err(|e| e.to_string())?.len();
        if bytes > 256 * 1024 * 1024 {
            return Err("sprite-sequence-too-large".into());
        }
        seen.insert(state, name.clone());
        Ok(name)
    };
    if let Some(frame) = preview {
        image_for(frame - start)?;
        return Ok(files);
    }
    let mut manifest = String::from("ffconcat version 1.0\n");
    let mut frame = 0;
    let mut count = 0;
    let mut last = String::new();
    while frame < duration {
        let name = image_for(frame)?;
        let length = sprite
            .next_change(offset + frame, start + frame, fps)
            .min(duration - frame);
        // concat's time base is microseconds: use differences of absolute rounded
        // boundaries so NTSC rates never accumulate per-entry truncation error.
        let micros = |f: u64| {
            (f as u128 * fps.denominator as u128 * 1_000_000).div_ceil(fps.numerator as u128)
        };
        let us = micros(frame + length) - micros(frame);
        manifest.push_str(&format!(
            "file '{name}'\noption framerate {}\nduration {}.{:06}\n",
            fps.ffmpeg(),
            us / 1_000_000,
            us % 1_000_000
        ));
        frame += length;
        count += 1;
        if count > 250_000 {
            return Err("sprite-sequence-too-large".into());
        }
        last = name;
    }
    // A final packet gives FFmpeg enough timestamp information to repeat the last state.
    manifest.push_str(&format!(
        "file '{last}'\noption framerate {}\n",
        fps.ffmpeg()
    ));
    let mut out = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(&files.input)
        .map_err(|e| e.to_string())?;
    out.write_all(manifest.as_bytes())
        .map_err(|e| e.to_string())?;
    files.animated = true;
    Ok(files)
}
