use super::ffmpeg::run;
use crate::{Result, project::*};
use serde_json::Value;
use std::{path::Path, process::Command, time::Duration};

fn number(value: Option<&Value>) -> Option<f64> {
    value
        .and_then(|v| v.as_f64().or_else(|| v.as_str()?.parse().ok()))
        .filter(|v| v.is_finite())
}
pub fn parse(bytes: &[u8], path: &Path) -> Result<MediaAsset> {
    crate::assets::require_utf8_path(path)?;
    let json: Value = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    let streams = json["streams"].as_array().ok_or("invalid-media")?;
    let video = streams
        .iter()
        .find(|s| s["codec_type"] == "video" && s["disposition"]["attached_pic"] != 1);
    let audio = streams.iter().find(|s| s["codec_type"] == "audio");
    if video.is_none() && audio.is_none() {
        return Err("unsupported-media".into());
    }
    let format = json["format"]["format_name"].as_str().unwrap_or_default();
    let image = video.is_some() && (format.contains("image2") || format.contains("_pipe"));
    let duration_seconds = number(json["format"].get("duration"))
        .or_else(|| {
            streams
                .iter()
                .filter_map(|s| number(s.get("duration")))
                .reduce(f64::max)
        })
        .unwrap_or(0.0);
    if !image && duration_seconds <= 0.0 {
        return Err("unknown-duration".into());
    }
    let frame_rate = video
        .and_then(|v| v["avg_frame_rate"].as_str())
        .and_then(|s| {
            let (n, d) = s.split_once('/')?;
            let rate = FrameRate {
                numerator: n.parse().ok()?,
                denominator: d.parse().ok()?,
            };
            rate.validate().ok().map(|()| rate)
        });
    let rotation = video
        .and_then(|v| v["side_data_list"].as_array())
        .and_then(|a| a.iter().find_map(|s| s["rotation"].as_i64()))
        .or_else(|| {
            video
                .and_then(|v| v["tags"]["rotate"].as_str())
                .and_then(|s| s.parse().ok())
        })
        .unwrap_or(0) as i32;
    Ok(MediaAsset {
        id: Id::new_v4(),
        name: path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into(),
        location: AssetLocation::External(path.to_path_buf()),
        kind: if image {
            MediaKind::Image
        } else if video.is_some() {
            MediaKind::Video
        } else {
            MediaKind::Audio
        },
        duration_seconds: duration_seconds.max(0.0),
        width: video.and_then(|v| v["width"].as_u64()).unwrap_or(0) as u32,
        height: video.and_then(|v| v["height"].as_u64()).unwrap_or(0) as u32,
        has_audio: audio.is_some(),
        sample_rate: audio
            .and_then(|v| v["sample_rate"].as_str())
            .and_then(|s| s.parse().ok()),
        frame_rate,
        codec: video
            .or(audio)
            .and_then(|v| v["codec_name"].as_str())
            .unwrap_or_default()
            .into(),
        start_time: number(json["format"].get("start_time")).unwrap_or(0.0),
        rotation,
        video_stream: video.and_then(|v| v["index"].as_u64()).map(|v| v as u32),
        audio_stream: audio.and_then(|v| v["index"].as_u64()).map(|v| v as u32),
    })
}
pub fn probe(binary: &Path, path: &Path) -> Result<MediaAsset> {
    crate::assets::require_utf8_path(path)?;
    let path = path.canonicalize().map_err(|e| e.to_string())?;
    crate::assets::require_utf8_path(&path)?;
    if !path.is_file() {
        return Err("invalid-media".into());
    }
    let output = run(
        Command::new(binary)
            .args([
                "-v",
                "error",
                "-protocol_whitelist",
                "file,pipe",
                "-show_streams",
                "-show_format",
                "-of",
                "json",
            ])
            .arg(&path),
        Duration::from_secs(30),
        || false,
    )?;
    parse(&output.stdout, &path)
}
