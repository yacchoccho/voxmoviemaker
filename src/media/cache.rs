use super::{ffmpeg::run, render_plan::RenderPlan};
use crate::{
    Result,
    assets::{Storage, resolved_path},
};
use std::{
    collections::hash_map::DefaultHasher,
    fs,
    hash::{Hash, Hasher},
    path::Path,
    process::Command,
    time::Duration,
};

pub fn preview(
    plan: &RenderPlan,
    storage: &Storage,
    ffmpeg: &Path,
    frame: u64,
    width: u32,
    limit_mb: u64,
    cancelled: impl Fn() -> bool,
) -> Result<image::RgbaImage> {
    let mut hash = DefaultHasher::new();
    serde_json::to_vec(plan)
        .map_err(|e| e.to_string())?
        .hash(&mut hash);
    frame.hash(&mut hash);
    width.hash(&mut hash);
    for clip in &plan.clips {
        if clip.visual
            && let Some(text) = &clip.text
        {
            super::text::hash_font(text, &mut hash)?;
        }
    }
    for clip in &plan.clips {
        if let Ok(m) = fs::metadata(&clip.path) {
            m.len().hash(&mut hash);
            m.modified().ok().hash(&mut hash);
        }
    }
    let directory = storage.cache.join("frames");
    let safe_cache = !resolved_path(&storage.cache)?.starts_with(resolved_path(&storage.data)?)
        && !resolved_path(&storage.data)?.starts_with(resolved_path(&storage.cache)?);
    if safe_cache {
        let limit = limit_mb.clamp(1, 4096) * 1024 * 1024;
        prune(&directory, limit);
        super::playback::prune(&storage.cache.join("proxies"), limit, Path::new(""));
    }
    let file = directory.join(format!("vmm-{:016x}.png", hash.finish()));
    if safe_cache
        && file.is_file()
        && let Ok(img) = read_image(&file)
    {
        return Ok(img);
    }
    let compiled = plan.compile(Some((frame, width)))?;
    let output = run(
        Command::new(ffmpeg).args(compiled.args).args([
            "-frames:v",
            "1",
            "-an",
            "-c:v",
            "png",
            "-f",
            "image2pipe",
            "pipe:1",
        ]),
        Duration::from_secs(30),
        cancelled,
    )?;
    let mut reader = image::ImageReader::with_format(
        std::io::Cursor::new(&output.stdout),
        image::ImageFormat::Png,
    );
    reader.limits(preview_limits());
    let img = reader.decode().map_err(|e| e.to_string())?.to_rgba8();
    if safe_cache && fs::create_dir_all(&directory).is_ok() {
        let _ = crate::project::persistence::atomic_write(&file, &output.stdout);
        prune(&directory, limit_mb.clamp(1, 4096) * 1024 * 1024);
    }
    Ok(img)
}
fn read_image(path: &Path) -> Result<image::RgbaImage> {
    let mut reader = image::ImageReader::open(path).map_err(|e| e.to_string())?;
    reader.limits(preview_limits());
    Ok(reader.decode().map_err(|e| e.to_string())?.to_rgba8())
}
fn preview_limits() -> image::Limits {
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(1280);
    limits.max_image_height = Some(720);
    limits.max_alloc = Some(8 * 1024 * 1024);
    limits
}
pub(crate) fn prune(directory: &Path, limit: u64) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    let mut files: Vec<_> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name();
            let name = name.to_str()?;
            if !name.starts_with("vmm-") || !name.ends_with(".png") {
                return None;
            }
            let m = e.metadata().ok()?;
            if !m.is_file() {
                return None;
            }
            Some((m.modified().ok()?, m.len(), e.path()))
        })
        .collect();
    let mut total: u64 = files.iter().map(|f| f.1).sum();
    files.sort_by_key(|f| f.0);
    for (_, size, path) in files {
        if total <= limit {
            break;
        }
        if fs::remove_file(path).is_ok() {
            total = total.saturating_sub(size);
        }
    }
}
