//! Opt-in test against a server already started by the user. Never starts/stops/downloads ENGINE.
use std::{
    fs,
    path::PathBuf,
    process::Command,
    sync::{Arc, atomic::AtomicBool},
    time::Duration,
};
use voxmoviemaker::{
    assets::Storage,
    media::{cache, export, ffmpeg, ffprobe, render_plan::RenderPlan},
    project::{persistence, *},
    settings::{ExportFormat, Settings},
    voicevox::{self, Client},
};
struct Fixture(PathBuf);
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
#[test]
#[ignore = "requires user-started VOICEVOX ENGINE and user-installed FFmpeg/ffprobe"]
fn live_speech_persistence_reload_preview_and_mp4_export() {
    let root = std::env::temp_dir().join(format!("vmm-live-{}", Id::new_v4()));
    fs::create_dir(&root).unwrap();
    let fixture = Fixture(root);
    let storage = Storage {
        data: fixture.0.join("data"),
        cache: fixture.0.join("cache"),
        config: fixture.0.join("config"),
    };
    let settings = Settings::default();
    let client = Client::new(&settings.voicevox_url, false).unwrap();
    let connection = client.connect().unwrap();
    let speaker = connection.speakers.first().expect("a talk speaker");
    let style = speaker.styles.first().unwrap();
    let voice = VoicevoxSettings {
        speaker: style.id,
        speaker_name: speaker.name.clone(),
        style_name: style.name.clone(),
        speed: 1.1,
        ..Default::default()
    };
    let text = "これは動画編集ソフトの動作確認です。";
    let (wav, seconds, sample_rate) = client.synthesize(text, &voice).unwrap();
    assert!(seconds > 0.1 && seconds < 30.0);
    assert_eq!(sample_rate, 48000);
    let mut project = Project {
        width: 64,
        height: 64,
        ..Default::default()
    };
    let audio = voicevox::persist(&storage, project.project_uuid, &wav, &voice).unwrap();
    let path = storage.resolve(project.project_uuid, &audio);
    assert_eq!(fs::read(&path).unwrap(), wav);
    assert!(
        path.starts_with(
            storage
                .data
                .join("projects")
                .join(project.project_uuid.to_string())
                .join("voice")
        )
    );
    let image_path = fixture.0.join("背景.png");
    image::RgbaImage::from_pixel(64, 64, image::Rgba([255, 0, 0, 255]))
        .save(&image_path)
        .unwrap();
    let image = ffprobe::probe(&settings.ffprobe, &image_path).unwrap();
    let duration = project.frame_rate.frames(seconds);
    project.tracks[0].clips.push(Clip {
        id: Id::new_v4(),
        source: ClipSource::Media { asset_id: image.id },
        timeline_start: 0,
        duration,
        source_offset: 0,
        volume: 1.0,
        muted: false,
        effects: Default::default(),
    });
    project.tracks[1].clips.push(Clip {
        id: Id::new_v4(),
        source: ClipSource::Voicevox {
            asset_id: audio.id,
            text: text.into(),
            settings: voice,
            subtitle: None,
        },
        timeline_start: 0,
        duration,
        source_offset: 0,
        volume: 1.0,
        muted: false,
        effects: Default::default(),
    });
    project.assets = vec![image, audio];
    let project_file = fixture.0.join("project.vmm.json");
    persistence::save(&project_file, &project).unwrap();
    let json = fs::read_to_string(&project_file).unwrap();
    assert!(!json.contains("RIFF"));
    assert!(!json.contains(".wav"));
    let loaded = persistence::load(&project_file).unwrap();
    assert_eq!(project, loaded);
    // Subsequent operations only read saved assets; the HTTP client is no longer used.
    drop(client);
    let plan = RenderPlan::new(&loaded, &storage).unwrap();
    let frame = cache::preview(&plan, &storage, &settings.ffmpeg, 0, 320, 8, || false).unwrap();
    let pixel = frame.get_pixel(160, 160);
    assert!(pixel[0] > 200 && pixel[2] < 40);
    let capabilities = ffmpeg::detect(&settings).unwrap();
    let output = fixture.0.join("voice.mp4");
    export::export(
        &plan,
        &settings.ffmpeg,
        &capabilities,
        ExportFormat::Mp4,
        &output,
        Arc::new(AtomicBool::new(false)),
        |_| {},
    )
    .unwrap();
    let info = ffprobe::probe(&settings.ffprobe, &output).unwrap();
    assert!(info.has_audio);
    assert!((info.duration_seconds - seconds).abs() < 0.1);
    let pcm = ffmpeg::run(
        Command::new(&settings.ffmpeg)
            .args(["-v", "error", "-i"])
            .arg(&output)
            .args(["-vn", "-ac", "1", "-f", "f32le", "pipe:1"]),
        Duration::from_secs(15),
        || false,
    )
    .unwrap()
    .stdout;
    assert!(
        pcm.as_chunks::<4>()
            .0
            .iter()
            .any(|b| f32::from_le_bytes(*b).abs() > 0.01)
    );
}
