use super::ffmpeg::{run, run_stream};
use crate::{Result, project::MediaKind};
use std::{
    io::{BufReader, Read},
    path::Path,
    process::Command,
    time::Duration,
};

pub enum Decoration {
    Image(image::RgbaImage),
    Waveform(Waveform),
}
pub struct Waveform {
    pub peaks: Vec<f32>,
    pub rms: Vec<f32>,
}
pub fn envelope(samples: impl Read, duration: f64) -> Waveform {
    let mut samples = BufReader::new(samples);
    let expected = (duration * 48_000.0).max(1.0);
    let mut bins = vec![0.0_f32; 2048];
    let mut energy = vec![0.0_f64; 2048];
    let mut counts = vec![0u64; 2048];
    let mut index = 0_u64;
    let mut bytes = [0; 4];
    while samples.read_exact(&mut bytes).is_ok() {
        let bin = ((index as f64 / expected * 2048.0) as usize).min(2047);
        let value = f32::from_le_bytes(bytes);
        if value.is_finite() {
            bins[bin] = bins[bin].max(value.abs().min(1.0));
            energy[bin] += (value as f64).clamp(-1.0, 1.0).powi(2);
            counts[bin] += 1;
        }
        index += 1;
    }
    Waveform {
        peaks: bins,
        rms: energy
            .iter()
            .zip(counts)
            .map(|(sum, count)| {
                if count == 0 {
                    0.0
                } else {
                    (sum / count as f64).sqrt() as f32
                }
            })
            .collect(),
    }
}
pub fn generate(ffmpeg: &Path, path: &Path, kind: MediaKind, duration: f64) -> Result<Decoration> {
    let path = path.canonicalize().map_err(|e| e.to_string())?;
    let mut command = Command::new(ffmpeg);
    command
        .args([
            "-hide_banner",
            "-nostdin",
            "-v",
            "error",
            "-threads",
            "1",
            "-protocol_whitelist",
            "file,pipe",
            "-i",
        ])
        .arg(path);
    if kind == MediaKind::Audio {
        command.args(["-vn", "-ac", "1", "-ar", "48000", "-f", "f32le", "pipe:1"]);
        // Constant memory, regardless of source duration. No complete waveform sample buffer.
        let (bins, _) = run_stream(
            &mut command,
            Duration::from_secs(15),
            || false,
            move |stdout| envelope(stdout, duration),
        )?;
        Ok(Decoration::Waveform(bins))
    } else {
        command.args([
            "-frames:v",
            "1",
            "-an",
            "-vf",
            "scale=96:54:force_original_aspect_ratio=decrease",
            "-c:v",
            "png",
            "-f",
            "image2pipe",
            "pipe:1",
        ]);
        let output = run(&mut command, Duration::from_secs(10), || false)?;
        Ok(Decoration::Image(
            image::load_from_memory(&output.stdout)
                .map_err(|e| e.to_string())?
                .to_rgba8(),
        ))
    }
}
