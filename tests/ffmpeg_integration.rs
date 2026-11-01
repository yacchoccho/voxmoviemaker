//! Opt-in real FFmpeg tests; no installation or download occurs.
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, atomic::AtomicBool},
    time::Duration,
};
use voxmoviemaker::{
    assets::Storage,
    media::{cache, export, ffmpeg, ffprobe, render_plan::RenderPlan},
    project::*,
    settings::{ExportFormat, Settings},
};
struct Fixture {
    directory: PathBuf,
    storage: Storage,
    settings: Settings,
}
impl Fixture {
    fn new() -> Self {
        let directory = std::env::temp_dir().join(format!("vmm-real-{}", Id::new_v4()));
        fs::create_dir(&directory).unwrap();
        let settings = Settings {
            ffmpeg: std::env::var_os("VMM_TEST_FFMPEG")
                .unwrap_or_else(|| "ffmpeg".into())
                .into(),
            ffprobe: std::env::var_os("VMM_TEST_FFPROBE")
                .unwrap_or_else(|| "ffprobe".into())
                .into(),
            ..Default::default()
        };
        let storage = Storage {
            data: directory.join("data"),
            cache: directory.join("cache"),
            config: directory.join("config"),
        };
        Self {
            directory,
            storage,
            settings,
        }
    }
    fn image(&self, name: &str, transparent: bool, color: [u8; 4]) -> MediaAsset {
        let path = self.directory.join(name);
        let mut image = image::RgbaImage::from_pixel(32, 32, image::Rgba(color));
        if transparent {
            for y in 0..32 {
                for x in 0..16 {
                    image.put_pixel(x, y, image::Rgba([0, 0, 0, 0]));
                }
            }
        }
        image.save(&path).unwrap();
        ffprobe::probe(&self.settings.ffprobe, &path).unwrap()
    }
    fn frame(&self, path: &Path) -> image::RgbaImage {
        let bytes = ffmpeg::run(
            Command::new(&self.settings.ffmpeg)
                .args(["-v", "error", "-i"])
                .arg(path)
                .args([
                    "-frames:v",
                    "1",
                    "-f",
                    "image2pipe",
                    "-c:v",
                    "png",
                    "pipe:1",
                ]),
            Duration::from_secs(15),
            || false,
        )
        .unwrap()
        .stdout;
        image::load_from_memory(&bytes).unwrap().to_rgba8()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}
fn add(p: &mut Project, a: MediaAsset, track: usize) {
    let c = Clip {
        id: Id::new_v4(),
        source: ClipSource::Media { asset_id: a.id },
        timeline_start: 0,
        duration: 30,
        source_offset: 0,
        volume: 1.0,
        muted: false,
        effects: Default::default(),
    };
    p.assets.push(a);
    p.tracks[track].clips.push(c);
}
fn is_red(pixel: &image::Rgba<u8>) -> bool {
    pixel[0] > 200 && pixel[2] < 40
}
fn is_blue(pixel: &image::Rgba<u8>) -> bool {
    pixel[2] > 200 && pixel[0] < 40
}
#[test]
#[ignore = "requires user-installed FFmpeg/ffprobe with libx264/aac and libvpx-vp9/libopus"]
fn actual_alpha_layer_order_reordering_and_preview_export_agree() {
    let f = Fixture::new();
    let caps = ffmpeg::detect(&f.settings).unwrap();
    let mut p = Project {
        width: 32,
        height: 32,
        ..Default::default()
    };
    add(&mut p, f.image("赤い背景.png", false, [255, 0, 0, 255]), 0);
    add(&mut p, f.image("透明;青.png", true, [0, 0, 255, 255]), 1);
    for format in [ExportFormat::Mp4, ExportFormat::WebM] {
        let plan = RenderPlan::new(&p, &f.storage).unwrap();
        let frame =
            cache::preview(&plan, &f.storage, &f.settings.ffmpeg, 0, 320, 8, || false).unwrap();
        assert!(is_red(frame.get_pixel(40, 160)));
        assert!(is_blue(frame.get_pixel(280, 160)));
        let output = f.directory.join(match format {
            ExportFormat::Mp4 => "out.mp4",
            ExportFormat::WebM => "out.webm",
        });
        export::export(
            &plan,
            &f.settings.ffmpeg,
            &caps,
            format,
            &output,
            Arc::new(AtomicBool::new(false)),
            |_| {},
        )
        .unwrap();
        let frame = f.frame(&output);
        assert!(is_red(frame.get_pixel(4, 16)));
        assert!(is_blue(frame.get_pixel(28, 16)));
    }
    let blue_id = p.assets[1].id;
    p.assets[1] = MediaAsset {
        id: blue_id,
        ..f.image("不透明な青.png", false, [0, 0, 255, 255])
    };
    let opaque_plan = RenderPlan::new(&p, &f.storage).unwrap();
    let opaque = cache::preview(
        &opaque_plan,
        &f.storage,
        &f.settings.ffmpeg,
        0,
        320,
        8,
        || false,
    )
    .unwrap();
    assert!(is_blue(opaque.get_pixel(40, 160)));
    assert!(is_blue(opaque.get_pixel(280, 160)));
    p.tracks.swap(0, 1);
    let plan = RenderPlan::new(&p, &f.storage).unwrap();
    let frame = cache::preview(&plan, &f.storage, &f.settings.ffmpeg, 0, 320, 8, || false).unwrap();
    assert!(is_red(frame.get_pixel(280, 160)));
    let reordered = f.directory.join("reordered.mp4");
    export::export(
        &plan,
        &f.settings.ffmpeg,
        &caps,
        ExportFormat::Mp4,
        &reordered,
        Arc::new(AtomicBool::new(false)),
        |_| {},
    )
    .unwrap();
    assert!(is_red(f.frame(&reordered).get_pixel(28, 16)));
}
#[test]
#[ignore = "requires user-installed FFmpeg/ffprobe with ffv1, libx264 and aac"]
fn covered_video_audio_and_voice_are_mixed_without_occluding_video() {
    let f = Fixture::new();
    let caps = ffmpeg::detect(&f.settings).unwrap();
    let wav = f.directory.join("voice.wav");
    let mut writer = hound::WavWriter::create(
        &wav,
        hound::WavSpec {
            channels: 1,
            sample_rate: 48000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        },
    )
    .unwrap();
    for sample in 0..48000 {
        writer
            .write_sample(
                (f64::sin(sample as f64 * std::f64::consts::TAU * 440.0 / 48000.0) * 4000.0) as i16,
            )
            .unwrap();
    }
    writer.finalize().unwrap();
    let waveform =
        voxmoviemaker::media::thumbnail::generate(&f.settings.ffmpeg, &wav, MediaKind::Audio, 1.0)
            .unwrap();
    let voxmoviemaker::media::thumbnail::Decoration::Waveform(bins) = waveform else {
        panic!("expected waveform")
    };
    assert!(bins.peaks.iter().all(|v| *v > 0.0 && *v <= 0.123));
    assert!(bins.peaks.iter().any(|v| *v > 0.12));
    let rms = (bins.rms.iter().map(|v| v * v).sum::<f32>() / bins.rms.len() as f32).sqrt();
    let expected = 4000.0_f32 / 32768.0 / 2.0_f32.sqrt();
    assert!((rms - expected).abs() < 0.002);
    let video = f.directory.join("video.mkv");
    ffmpeg::run(
        Command::new(&f.settings.ffmpeg)
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "color=c=red:s=32x32:r=30:d=1",
                "-i",
            ])
            .arg(&wav)
            .args(["-c:v", "ffv1", "-c:a", "pcm_s16le", "-t", "1"])
            .arg(&video),
        Duration::from_secs(15),
        || false,
    )
    .unwrap();
    let mut p = Project {
        width: 32,
        height: 32,
        ..Default::default()
    };
    add(
        &mut p,
        ffprobe::probe(&f.settings.ffprobe, &video).unwrap(),
        0,
    );
    add(&mut p, f.image("blue.png", false, [0, 0, 255, 255]), 1);
    add(
        &mut p,
        ffprobe::probe(&f.settings.ffprobe, &wav).unwrap(),
        2,
    );
    let output = f.directory.join("mix.mp4");
    let plan = RenderPlan::new(&p, &f.storage).unwrap();
    export::export(
        &plan,
        &f.settings.ffmpeg,
        &caps,
        ExportFormat::Mp4,
        &output,
        Arc::new(AtomicBool::new(false)),
        |_| {},
    )
    .unwrap();
    assert!(is_blue(f.frame(&output).get_pixel(16, 16)));
    let pcm = ffmpeg::run(
        Command::new(&f.settings.ffmpeg)
            .args(["-v", "error", "-i"])
            .arg(&output)
            .args(["-vn", "-ac", "1", "-ar", "48000", "-f", "f32le", "pipe:1"]),
        Duration::from_secs(15),
        || false,
    )
    .unwrap()
    .stdout;
    let samples: Vec<_> = pcm
        .as_chunks::<4>()
        .0
        .iter()
        .map(|c| f32::from_le_bytes(*c) as f64)
        .collect();
    let rms = (samples.iter().map(|s| s * s).sum::<f64>() / samples.len() as f64).sqrt();
    assert!(rms > 0.12 && rms < 0.25, "RMS {rms}");
}
