use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    process::Command,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use voxmoviemaker::{
    assets::Storage,
    media::{
        export::export,
        ffmpeg::{self, Capabilities},
        render_plan::RenderPlan,
        thumbnail::envelope,
    },
    project::*,
    settings::ExportFormat,
};
struct Fixture {
    root: PathBuf,
    plan: RenderPlan,
    binary: PathBuf,
    target: PathBuf,
    caps: Capabilities,
}
impl Fixture {
    fn new(script: &str) -> Self {
        let root = std::env::temp_dir().join(format!("vmm-process-{}", Id::new_v4()));
        fs::create_dir(&root).unwrap();
        let binary = root.join("fake-ffmpeg");
        fs::write(&binary, format!("#!/bin/sh\n{script}\n")).unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
        let source = root.join("source.png");
        fs::write(&source, b"source").unwrap();
        let a = MediaAsset {
            id: Id::new_v4(),
            name: "source".into(),
            location: AssetLocation::External(source),
            kind: MediaKind::Image,
            duration_seconds: 1.0,
            width: 32,
            height: 32,
            has_audio: false,
            sample_rate: None,
            frame_rate: None,
            codec: "png".into(),
            start_time: 0.0,
            rotation: 0,
            video_stream: Some(0),
            audio_stream: None,
        };
        let mut p = Project::default();
        p.tracks[0].clips.push(Clip {
            id: Id::new_v4(),
            source: ClipSource::Media { asset_id: a.id },
            timeline_start: 0,
            duration: 30,
            source_offset: 0,
            volume: 1.0,
            muted: false,
            effects: Default::default(),
        });
        p.assets.push(a);
        let storage = Storage {
            data: root.join("data"),
            cache: root.join("cache"),
            config: root.join("config"),
        };
        let plan = RenderPlan::new(&p, &storage).unwrap();
        let target = root.join("output.mp4");
        let caps = Capabilities {
            encoders: " V..... libx264\n A..... aac".into(),
            formats: " E mp4".into(),
            ..Default::default()
        };
        Self {
            root,
            plan,
            binary,
            target,
            caps,
        }
    }
    fn leftovers(&self) -> usize {
        fs::read_dir(&self.root)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().ends_with(".part"))
            .count()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
#[test]
fn external_process_timeout_and_cancellation_are_reaped() {
    let start = Instant::now();
    let result = ffmpeg::run(
        Command::new("sleep").arg("10"),
        Duration::from_millis(30),
        || false,
    );
    assert_eq!(result.err().unwrap(), "process-timeout");
    assert!(start.elapsed() < Duration::from_secs(2));
    let result = ffmpeg::run(
        Command::new("sleep").arg("10"),
        Duration::from_secs(20),
        || true,
    );
    assert_eq!(result.err().unwrap(), "cancelled");
}
#[test]
fn export_publication_progress_and_existing_file_protection() {
    let f = Fixture::new(
        "for last do :; done\nprintf 'out_time_us=500000\\n'\nprintf 'encoded-video' > \"$last\"",
    );
    let progress = Mutex::new(Vec::new());
    export(
        &f.plan,
        &f.binary,
        &f.caps,
        ExportFormat::Mp4,
        &f.target,
        Arc::new(AtomicBool::new(false)),
        |v| progress.lock().unwrap().push(v),
    )
    .unwrap();
    assert_eq!(fs::read(&f.target).unwrap(), b"encoded-video");
    assert_eq!(progress.lock().unwrap().last(), Some(&1.0));
    assert_eq!(f.leftovers(), 0);
    let result = export(
        &f.plan,
        &f.binary,
        &f.caps,
        ExportFormat::Mp4,
        &f.target,
        Arc::new(AtomicBool::new(false)),
        |_| {},
    );
    assert_eq!(result.unwrap_err(), "output-exists");
    assert_eq!(fs::read(&f.target).unwrap(), b"encoded-video");
}
#[test]
fn failed_and_cancelled_export_never_publish_partial_output() {
    let f = Fixture::new("for last do :; done\nprintf 'partial' > \"$last\"\nexit 1");
    assert!(
        export(
            &f.plan,
            &f.binary,
            &f.caps,
            ExportFormat::Mp4,
            &f.target,
            Arc::new(AtomicBool::new(false)),
            |_| {}
        )
        .is_err()
    );
    assert!(!f.target.exists());
    assert_eq!(f.leftovers(), 0);
    let f = Fixture::new("exec sleep 10");
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = cancel.clone();
    let handle = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(60));
        flag.store(true, Ordering::Relaxed);
    });
    let start = Instant::now();
    let result = export(
        &f.plan,
        &f.binary,
        &f.caps,
        ExportFormat::Mp4,
        &f.target,
        cancel,
        |_| {},
    );
    handle.join().unwrap();
    assert_eq!(result.unwrap_err(), "cancelled");
    assert!(!f.target.exists());
    assert_eq!(f.leftovers(), 0);
    assert!(start.elapsed() < Duration::from_secs(2));
}
#[test]
fn waveform_uses_a_fixed_envelope_and_rejects_nan() {
    let mut bytes = Vec::new();
    for i in 0..48000 {
        bytes.extend_from_slice(&(if i == 20 { f32::NAN } else { 0.5_f32 }).to_le_bytes());
    }
    let bins = envelope(bytes.as_slice(), 1.0);
    assert_eq!(bins.peaks.len(), 2048);
    assert!(bins.peaks.iter().chain(&bins.rms).all(|v| v.is_finite()));
    assert_eq!(bins.peaks.iter().copied().fold(0.0_f32, f32::max), 0.5);
    assert!(
        bins.rms
            .iter()
            .zip(&bins.peaks)
            .all(|(rms, peak)| rms <= peak)
    );
}
