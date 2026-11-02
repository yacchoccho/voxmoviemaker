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
        character: None,
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
        character: None,
        id: Id::new_v4(),
        source: ClipSource::Voicevox {
            asset_id: audio.id,
            text: text.into(),
            settings: voice.into(),
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

#[test]
#[ignore = "requires user-started VOICEVOX ENGINE and user-installed FFmpeg/ffprobe"]
fn live_character_generation_regeneration_shared_subtitles_and_mp4_export() {
    use voxmoviemaker::{app::App, characters::*};
    let root = std::env::temp_dir().join(format!("vmm-live-character-{}", Id::new_v4()));
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
    let speaker = connection.speakers.first().unwrap();
    let style = speaker.styles.first().unwrap();
    let mut character = Character::new("Live character".into(), 320, 180);
    character.voice = VoicevoxSettings {
        speaker: style.id,
        speaker_uuid: speaker.speaker_uuid.clone(),
        speaker_name: speaker.name.clone(),
        style_name: style.name.clone(),
        pause_length: Some(0.25),
        pause_scale: Some(1.1),
        sampling_rate: 24000,
        stereo: true,
        interrogative_upspeak: false,
        ..Default::default()
    };
    character.subtitle.size = 24.0;
    let id = character.id;
    let ctx = eframe::egui::Context::default();
    let mut app = App::from_context(ctx.clone(), storage.clone(), settings.clone(), None);
    app.project.width = 320;
    app.project.height = 180;
    app.characters
        .save(CharacterLibrary {
            schema_version: 1,
            characters: vec![character],
        })
        .unwrap();
    app.connection = Some(connection);
    app.settings.character = Some(id);
    app.voice_text = "確認です。".into();
    app.generate(false);
    let wait = |app: &mut App| {
        let deadline = std::time::Instant::now() + Duration::from_secs(60);
        while app.generating && std::time::Instant::now() < deadline {
            app.poll(&ctx);
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(!app.generating, "{}", app.status);
    };
    wait(&mut app);
    let clip_id = app.selected.expect("generated character clip");
    let first = app.project.clip(clip_id).unwrap().clone();
    assert_eq!(first.character.as_ref().unwrap().character_id, id);
    let first_path = storage.resolve(
        app.project.project_uuid,
        app.project.asset(first.source.asset_id().unwrap()).unwrap(),
    );
    let original_wav = fs::read(&first_path).unwrap();
    let spec = hound::WavReader::open(&first_path).unwrap().spec();
    assert_eq!(spec.sample_rate, 24000);
    assert_eq!(spec.channels, 2);
    let mut library = app.characters.library.clone();
    library.characters[0].voice.speed = 1.5;
    library.characters[0].subtitle.size = 20.0;
    app.save_characters(library).unwrap();
    assert_eq!(app.project.clip(clip_id), Some(&first));
    assert_eq!(
        fs::read(&first_path).unwrap(),
        original_wav,
        "changing defaults must preserve old audio"
    );
    app.project
        .clip_mut(clip_id)
        .unwrap()
        .character
        .as_mut()
        .unwrap()
        .voice
        .speed = Some(1.3);
    app.project
        .clip_mut(clip_id)
        .unwrap()
        .character
        .as_mut()
        .unwrap()
        .subtitle
        .x = Some(140.0);
    app.voice_text = "確認です。".into();
    app.regenerate_voice();
    wait(&mut app);
    let regenerated = app.project.clip(clip_id).unwrap();
    let second_path = storage.resolve(
        app.project.project_uuid,
        app.project
            .asset(regenerated.source.asset_id().unwrap())
            .unwrap(),
    );
    assert_ne!(first_path, second_path);
    assert_eq!(fs::read(&first_path).unwrap(), original_wav);
    assert_eq!(
        regenerated
            .character
            .as_ref()
            .unwrap()
            .voice(&app.characters.library)
            .unwrap()
            .speed,
        1.3
    );
    let project_file = fixture.0.join("characters.vmm.json");
    persistence::save(&project_file, &app.project).unwrap();
    let json = fs::read_to_string(&project_file).unwrap();
    assert!(!json.contains("Live character"));
    assert!(!json.contains("speaker_uuid"));
    let loaded = persistence::load(&project_file).unwrap();
    let plan = RenderPlan::new(&loaded, &storage).unwrap();
    let subtitle = plan.clips.iter().find_map(|c| c.text.as_ref()).unwrap();
    assert_eq!(subtitle.size, 20.0);
    assert_eq!(subtitle.position, Some([140.0, 148.0]));
    let preview = cache::preview(&plan, &storage, &settings.ffmpeg, 0, 320, 8, || false).unwrap();
    assert!(preview.pixels().any(|p| p[0] > 128));
    let caps = ffmpeg::detect(&settings).unwrap();
    let output = fixture.0.join("character.mp4");
    export::export(
        &plan,
        &settings.ffmpeg,
        &caps,
        ExportFormat::Mp4,
        &output,
        Arc::new(AtomicBool::new(false)),
        |_| {},
    )
    .unwrap();
    let info = ffprobe::probe(&settings.ffprobe, &output).unwrap();
    assert!(info.has_audio);
    let actual = ffmpeg::run(
        Command::new(&settings.ffmpeg)
            .args(["-v", "error", "-i"])
            .arg(&output)
            .args([
                "-frames:v",
                "1",
                "-f",
                "image2pipe",
                "-vcodec",
                "png",
                "pipe:1",
            ]),
        Duration::from_secs(15),
        || false,
    )
    .unwrap();
    let frame = image::load_from_memory(&actual.stdout).unwrap().to_rgba8();
    let error: f64 = preview
        .pixels()
        .zip(frame.pixels())
        .map(|(a, b)| a[0].abs_diff(b[0]) as f64)
        .sum::<f64>()
        / (320.0 * 180.0);
    assert!(error < 5.0, "preview/export error: {error}");
    let retained = app.project.clip(clip_id).unwrap().clone();
    let retained_wav = fs::read(&second_path).unwrap();
    app.settings.voicevox_url = "invalid-url".into();
    app.voice_text = "失敗時の保護を確認".into();
    app.regenerate_voice();
    wait(&mut app);
    assert_eq!(app.project.clip(clip_id), Some(&retained));
    assert_eq!(fs::read(&second_path).unwrap(), retained_wav);
    app.characters.save(CharacterLibrary::default()).unwrap();
    let missing = RenderPlan::new(&loaded, &storage).unwrap();
    assert!(
        missing
            .clips
            .iter()
            .any(|c| c.audio && c.path == second_path)
    );
    assert!(second_path.is_file());
}
