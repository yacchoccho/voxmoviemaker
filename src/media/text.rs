//! Original Rust text rendering. System fonts are read, never bundled or copied to projects.
use crate::{
    Result,
    assets::{private_directory, resolved_path},
    project::{TextAlign, TextAnchor, TextSettings},
};
use ab_glyph::{Font, FontArc, FontVec, PxScale, ScaleFont, point};
use std::{
    fs,
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    process::Command,
    sync::{Mutex, OnceLock},
    time::{Duration, SystemTime},
};

type CachedFont = (PathBuf, SystemTime, u64, u32, FontArc);
pub fn hash_font(settings: &TextSettings, state: &mut impl std::hash::Hasher) -> Result<()> {
    use std::hash::Hash;
    if settings.text.is_empty() {
        return Ok(());
    }
    let path = font_path(settings)?;
    let metadata = fs::metadata(&path).map_err(|e| format!("text-font-missing: {e}"))?;
    path.hash(state);
    metadata.len().hash(state);
    metadata.modified().ok().hash(state);
    Ok(())
}
static FONTS: OnceLock<Mutex<Vec<CachedFont>>> = OnceLock::new();
pub fn font_path(settings: &TextSettings) -> Result<PathBuf> {
    if !settings.font.as_os_str().is_empty() {
        return Ok(settings.font.clone());
    }
    static DEFAULT: OnceLock<Option<PathBuf>> = OnceLock::new();
    DEFAULT
        .get_or_init(|| {
            let matched = super::ffmpeg::run(
                Command::new("fc-match").args(["-f", "%{file}", "sans:lang=ja"]),
                Duration::from_secs(3),
                || false,
            )
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| PathBuf::from(s.trim()));
            matched
                .into_iter()
                .chain([
                    PathBuf::from("/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc"),
                    PathBuf::from("/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf"),
                ])
                .find(|p| p.is_file())
        })
        .clone()
        .ok_or_else(|| "text-font-missing".into())
}
fn load_font(settings: &TextSettings) -> Result<FontArc> {
    let path = font_path(settings)?;
    let metadata = fs::metadata(&path).map_err(|e| format!("text-font-missing: {e}"))?;
    if metadata.len() > 64 * 1024 * 1024 {
        return Err("text-font-invalid".into());
    }
    let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
    let cache = FONTS.get_or_init(|| Mutex::new(Vec::new()));
    let mut cache = cache.lock().map_err(|_| "text-font-invalid")?;
    if let Some((_, _, _, _, font)) = cache.iter().find(|(p, m, n, i, _)| {
        p == &path && m == &modified && *n == metadata.len() && *i == settings.font_index
    }) {
        return Ok(font.clone());
    }
    let bytes = fs::read(&path).map_err(|e| e.to_string())?;
    let font: FontArc = FontVec::try_from_vec_and_index(bytes, settings.font_index)
        .map_err(|_| "text-font-invalid")?
        .into();
    if cache.len() >= 2 {
        cache.remove(0);
    }
    cache.push((
        path,
        modified,
        metadata.len(),
        settings.font_index,
        font.clone(),
    ));
    Ok(font)
}
pub struct TextImage {
    pub path: PathBuf,
}
impl Drop for TextImage {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}
pub fn temporary_image(
    settings: &TextSettings,
    width: u32,
    height: u32,
    project_width: u32,
    scratch: &Path,
    data: &Path,
) -> Result<TextImage> {
    let cache = resolved_path(scratch)?;
    let data = resolved_path(data)?;
    if cache.starts_with(&data) || data.starts_with(&cache) {
        return Err("data-cache-overlap".into());
    }
    private_directory(scratch)?;
    let image = rasterize(settings, width, height, project_width)?;
    let path = scratch.join(format!("vmm-text-{}.png", uuid::Uuid::new_v4()));
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(&path)
        .map_err(|e| e.to_string())?;
    let guard = TextImage { path };
    image
        .write_to(&mut file, image::ImageFormat::Png)
        .map_err(|e| e.to_string())?;
    Ok(guard)
}
pub fn rasterize(
    settings: &TextSettings,
    width: u32,
    height: u32,
    project_width: u32,
) -> Result<image::RgbaImage> {
    if width == 0 || height == 0 || project_width == 0 || settings.text.chars().count() > 4096 {
        return Err("invalid-text".into());
    }
    // Limit transient Rust raster memory even when the final export is 8K.
    let ratio = (3840.0 / width as f32).min(2160.0 / height as f32).min(1.0);
    let w = (width as f32 * ratio).round().max(1.0) as u32;
    let h = (height as f32 * ratio).round().max(1.0) as u32;
    if settings.text.is_empty() {
        return Ok(image::RgbaImage::new(w, h));
    }
    let unit = w as f32 / project_width as f32;
    let font = load_font(settings)?;
    let scaled = font.as_scaled(PxScale::from((settings.size * unit).max(1.0)));
    let padding = (32.0 * unit)
        .max(1.0)
        .min(w as f32 / 4.0)
        .min(h as f32 / 4.0);
    let line_height = (scaled.height() + scaled.line_gap()).max(1.0);
    let max_width = (w as f32 - 2.0 * padding).max(1.0);
    let mut lines = vec![(Vec::new(), 0.0f32)];
    let mut previous = None;
    for ch in settings.text.chars().filter(|ch| *ch != '\r') {
        if ch == '\n' {
            lines.push((Vec::new(), 0.0));
            previous = None;
            continue;
        }
        let id = scaled.glyph_id(ch);
        let advance = scaled.h_advance(id);
        let kern = previous.map_or(0.0, |prev| scaled.kern(prev, id));
        let line = lines.last_mut().ok_or("invalid-text")?;
        if line.1 + kern + advance > max_width && !line.0.is_empty() {
            lines.push((Vec::new(), 0.0));
            previous = None;
        }
        let line = lines.last_mut().ok_or("invalid-text")?;
        line.1 += previous.map_or(0.0, |prev| scaled.kern(prev, id));
        line.0.push((id, line.1));
        line.1 += advance;
        previous = Some(id);
    }
    let total_height = lines.len() as f32 * line_height;
    if total_height > h as f32 - 2.0 * padding {
        return Err("text-layout-overflow".into());
    }
    let top = match settings.anchor {
        TextAnchor::Top => padding,
        TextAnchor::Center => (h as f32 - total_height) / 2.0,
        TextAnchor::Bottom => h as f32 - padding - total_height,
    };
    let mut mask = vec![0u8; w as usize * h as usize];
    for (row, (glyphs, line_width)) in lines.into_iter().enumerate() {
        let left = match settings.align {
            TextAlign::Left => padding,
            TextAlign::Center => (w as f32 - line_width) / 2.0,
            TextAlign::Right => w as f32 - padding - line_width,
        };
        for (id, x) in glyphs {
            let glyph = id.with_scale_and_position(
                scaled.scale(),
                point(left + x, top + scaled.ascent() + row as f32 * line_height),
            );
            if let Some(outline) = font.outline_glyph(glyph) {
                let bounds = outline.px_bounds();
                outline.draw(|x, y, coverage| {
                    let x = x as i32 + bounds.min.x as i32;
                    let y = y as i32 + bounds.min.y as i32;
                    if x >= 0 && y >= 0 && x < w as i32 && y < h as i32 {
                        let index = y as usize * w as usize + x as usize;
                        mask[index] = mask[index].max((coverage * 255.0).round() as u8);
                    }
                });
            }
        }
    }
    let radius = (settings.outline_width as f32 * unit).round() as i32;
    let mut border = mask.clone();
    if radius > 0 {
        for y in 0..h as i32 {
            for x in 0..w as i32 {
                let alpha = mask[y as usize * w as usize + x as usize];
                if alpha == 0 {
                    continue;
                }
                for dy in -radius..=radius {
                    for dx in -radius..=radius {
                        let (nx, ny) = (x + dx, y + dy);
                        if dx * dx + dy * dy <= radius * radius
                            && nx >= 0
                            && ny >= 0
                            && nx < w as i32
                            && ny < h as i32
                        {
                            let index = ny as usize * w as usize + nx as usize;
                            border[index] = border[index].max(alpha);
                        }
                    }
                }
            }
        }
    }
    Ok(image::RgbaImage::from_fn(w, h, |x, y| {
        let index = y as usize * w as usize + x as usize;
        let a = mask[index] as f32 / 255.0 * settings.color[3] as f32 / 255.0;
        let b = if radius > 0 {
            border[index] as f32 / 255.0 * settings.outline[3] as f32 / 255.0
        } else {
            0.0
        };
        let combined = a + b * (1.0 - a);
        if combined == 0.0 {
            return image::Rgba([0; 4]);
        }
        let mut pixel = [0; 4];
        for (channel, target) in pixel[..3].iter_mut().enumerate() {
            *target = ((settings.color[channel] as f32 * a
                + settings.outline[channel] as f32 * b * (1.0 - a))
                / combined)
                .round() as u8;
        }
        pixel[3] = (combined * 255.0).round() as u8;
        image::Rgba(pixel)
    }))
}
