use std::{
    fs,
    io::{Cursor, Read, Write},
    net::TcpListener,
    path::{Path, PathBuf},
    thread,
};
use voxmoviemaker::{
    assets::{Storage, ensure_outside_git, xdg_path},
    i18n::I18n,
    media::{ffmpeg::Capabilities, ffprobe, render_plan::RenderPlan},
    project::{
        commands::{self, History},
        persistence, *,
    },
    settings::{Settings, Theme},
    ui::timeline,
    voicevox::{self, Client},
};

struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("vmm-test-{}", Id::new_v4()));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
    fn storage(&self) -> Storage {
        Storage {
            data: self.path("data"),
            cache: self.path("cache"),
            config: self.path("config"),
        }
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn asset(temp: &Temp, name: &str, kind: MediaKind) -> MediaAsset {
    let path = temp.path(name);
    fs::write(&path, []).unwrap();
    MediaAsset {
        id: Id::new_v4(),
        name: name.into(),
        location: AssetLocation::External(path),
        kind,
        duration_seconds: 10.0,
        width: 32,
        height: 32,
        has_audio: kind != MediaKind::Image,
        sample_rate: Some(48000),
        frame_rate: None,
        codec: "test".into(),
        start_time: 0.0,
        rotation: 0,
        video_stream: None,
        audio_stream: None,
    }
}
fn clip(asset: &MediaAsset, start: u64, duration: u64) -> Clip {
    Clip {
        character: None,
        id: Id::new_v4(),
        source: ClipSource::Media { asset_id: asset.id },
        timeline_start: start,
        duration,
        source_offset: 0,
        volume: 1.0,
        muted: false,
        effects: Default::default(),
    }
}
fn scene(temp: &Temp) -> Project {
    let mut p = Project::default();
    for (index, name, kind) in [
        (0, "red.png", MediaKind::Image),
        (1, "blue.png", MediaKind::Image),
        (2, "voice.wav", MediaKind::Audio),
    ] {
        let a = asset(temp, name, kind);
        p.tracks[index].clips.push(clip(&a, 0, 30));
        p.assets.push(a);
    }
    p
}
#[test]
fn rational_time_and_samples_do_not_accumulate_error() {
    let ntsc = FrameRate {
        numerator: 30000,
        denominator: 1001,
    };
    assert_eq!(ntsc.timestamp(30000), "1001.000000000");
    assert_eq!(ntsc.samples(30000, 48000), 48_048_000);
    let cinema = FrameRate {
        numerator: 24000,
        denominator: 1001,
    };
    assert_eq!(cinema.timestamp(24000), "1001.000000000");
    assert!(
        FrameRate {
            numerator: 0,
            denominator: 1
        }
        .validate()
        .is_err()
    );
}
#[test]
fn lower_layers_are_foreground_and_audio_never_occludes() {
    let temp = Temp::new();
    let mut p = scene(&temp);
    let red = p.tracks[0].clips[0].id;
    let blue = p.tracks[1].clips[0].id;
    let plan = RenderPlan::new(&p, &temp.storage()).unwrap();
    assert_eq!(plan.visual_order(0), [red, blue]);
    assert!(plan.clips[2].audio);
    assert!(!plan.clips[2].visual);
    p.tracks.swap(0, 1);
    assert_eq!(
        RenderPlan::new(&p, &temp.storage())
            .unwrap()
            .visual_order(0),
        [blue, red]
    );
    assert!(plan.visual_order(30).is_empty());
}
#[test]
fn preview_export_share_overlay_order_and_alpha_filters() {
    let temp = Temp::new();
    let p = scene(&temp);
    let plan = RenderPlan::new(&p, &temp.storage()).unwrap();
    for preview in [None, Some((0, 320))] {
        let compiled = plan.compile(preview).unwrap();
        let args: Vec<_> = compiled.args.iter().map(|a| a.to_string_lossy()).collect();
        let index = args.iter().position(|a| a == "-filter_complex").unwrap();
        let graph = &args[index + 1];
        assert!(graph.contains("[base][v0]overlay"));
        assert!(graph.contains("[o0][v1]overlay"));
        assert!(graph.contains("format=rgba"));
        assert!(graph.contains("color=black@0"));
        if preview.is_none() {
            assert!(graph.contains("amix=inputs=1"));
        }
    }
}
#[test]
fn portrait_and_extreme_aspect_previews_stay_within_pixel_budget() {
    let temp = Temp::new();
    let mut p = scene(&temp);
    for (width, height) in [(2, 4320), (1080, 1920), (7680, 2)] {
        p.width = width;
        p.height = height;
        let frame = RenderPlan::new(&p, &temp.storage())
            .unwrap()
            .compile(Some((0, 1280)))
            .unwrap();
        assert!((2..=1280).contains(&frame.width));
        assert!((2..=720).contains(&frame.height));
        assert!(frame.width.is_multiple_of(2));
        assert!(frame.height.is_multiple_of(2));
    }
}
#[test]
fn audio_from_hidden_video_is_still_mixed_and_mutes_apply() {
    let temp = Temp::new();
    let mut p = scene(&temp);
    p.assets[0].kind = MediaKind::Video;
    p.assets[0].has_audio = true;
    p.tracks[0].video_visible = false;
    let plan = RenderPlan::new(&p, &temp.storage()).unwrap();
    assert!(!plan.clips[0].visual);
    assert!(plan.clips[0].audio);
    p.tracks[0].muted = true;
    assert!(!RenderPlan::new(&p, &temp.storage()).unwrap().clips[0].audio);
}
#[test]
fn mixed_media_on_generic_layer_and_collision_transactions() {
    let temp = Temp::new();
    let mut p = Project::default();
    let mut history = History::default();
    for (i, kind) in [MediaKind::Video, MediaKind::Image, MediaKind::Audio]
        .into_iter()
        .enumerate()
    {
        let a = asset(&temp, &format!("{i}"), kind);
        let c = clip(&a, i as u64 * 30, 30);
        history
            .edit(&mut p, |p| {
                p.assets.push(a);
                commands::insert(p, 0, c)
            })
            .unwrap();
    }
    let before = p.clone();
    let mut overlapping = p.tracks[0].clips[0].clone();
    overlapping.id = Id::new_v4();
    assert!(
        history
            .edit(&mut p, |p| commands::insert(p, 0, overlapping))
            .is_err()
    );
    assert_eq!(p, before);
    history.undo(&mut p);
    assert_eq!(p.tracks[0].clips.len(), 2);
    history.redo(&mut p);
    assert_eq!(p, before);
}
#[test]
fn move_split_trim_and_undo_preserve_source_offsets() {
    let temp = Temp::new();
    let mut p = scene(&temp);
    let id = p.tracks[0].clips[0].id;
    let mut h = History::default();
    h.edit(&mut p, |p| commands::split(p, id, 10)).unwrap();
    assert_eq!(p.tracks[0].clips[0].duration, 10);
    assert_eq!(p.tracks[0].clips[1].source_offset, 10);
    let right = p.tracks[0].clips[1].id;
    h.edit(&mut p, |p| commands::move_clip(p, right, 3, 50))
        .unwrap();
    assert_eq!(p.clip(right).unwrap().source_offset, 10);
    h.edit(&mut p, |p| {
        p.clip_mut(right).unwrap().duration = 5;
        Ok(())
    })
    .unwrap();
    assert_eq!(p.clip(right).unwrap().duration, 5);
    h.undo(&mut p);
    assert_eq!(p.clip(right).unwrap().duration, 20);
    h.undo(&mut p);
    assert_eq!(p.track_of(right), Some(0));
    h.undo(&mut p);
    assert_eq!(p.tracks[0].clips.len(), 1);
}
#[test]
fn locked_track_rejects_move_split_delete_without_changing_project() {
    let temp = Temp::new();
    let mut p = scene(&temp);
    p.tracks[0].locked = true;
    let original = p.clone();
    let id = p.tracks[0].clips[0].id;
    let mut h = History::default();
    assert!(
        h.edit(&mut p, |p| commands::move_clip(p, id, 3, 0))
            .is_err()
    );
    assert!(h.edit(&mut p, |p| commands::split(p, id, 15)).is_err());
    assert!(h.edit(&mut p, |p| commands::delete(p, id)).is_err());
    assert_eq!(p, original);
}
#[test]
fn timeline_drop_respects_zoom_and_both_scroll_offsets() {
    let canvas = eframe::egui::Rect::from_min_size(
        eframe::egui::pos2(-200.0, -80.0),
        eframe::egui::vec2(2000.0, 800.0),
    );
    let point = eframe::egui::pos2(
        -200.0 + timeline::LABEL_WIDTH + 250.0,
        -80.0 + timeline::RULER_HEIGHT + 2.5 * timeline::ROW_HEIGHT,
    );
    assert_eq!(
        timeline::position(point, canvas, FrameRate::default(), 100.0, 5),
        (2, 75)
    );
    assert_eq!(
        timeline::position(point, canvas, FrameRate::default(), 50.0, 5),
        (2, 150)
    );
    assert_eq!(
        timeline::position(
            eframe::egui::pos2(0.0, 900.0),
            canvas,
            FrameRate::default(),
            100.0,
            5
        )
        .0,
        5
    );
}
#[test]
fn json_roundtrip_keeps_order_ids_and_rejects_bad_schema() {
    let temp = Temp::new();
    let mut p = scene(&temp);
    p.tracks.swap(0, 1);
    let path = temp.path("project.vmm.json");
    persistence::save(&path, &p).unwrap();
    assert_eq!(persistence::load(&path).unwrap(), p);
    let mut invalid = p.clone();
    invalid.schema_version = 200;
    fs::write(&path, serde_json::to_vec(&invalid).unwrap()).unwrap();
    assert!(persistence::load(&path).is_err());
    fs::write(&path, "{broken").unwrap();
    assert!(persistence::load(&path).is_err());
}
#[test]
fn xdg_rejects_relative_values_and_git_paths_including_symlinks() {
    let temp = Temp::new();
    assert_eq!(
        xdg_path(
            Some("relative".into()),
            Path::new("/home/test"),
            ".local/share"
        ),
        PathBuf::from("/home/test/.local/share/voxmoviemaker")
    );
    let repo = temp.path("repository");
    fs::create_dir_all(repo.join(".git")).unwrap();
    assert_eq!(
        ensure_outside_git(&repo.join("data/new")).unwrap_err(),
        "voice-storage-in-git"
    );
    std::os::unix::fs::symlink(&repo, temp.path("link")).unwrap();
    assert!(ensure_outside_git(&temp.path("link/new")).is_err());
    fs::write(temp.path(".git"), "gitdir: /some/worktree").unwrap();
    assert!(ensure_outside_git(&temp.path("new")).is_err());
}
fn wav() -> Vec<u8> {
    let mut cursor = Cursor::new(Vec::new());
    {
        let mut writer = hound::WavWriter::new(
            &mut cursor,
            hound::WavSpec {
                channels: 1,
                sample_rate: 48000,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            },
        )
        .unwrap();
        for _ in 0..4800 {
            writer.write_sample::<i16>(1000).unwrap();
        }
        writer.finalize().unwrap();
    }
    cursor.into_inner()
}
#[test]
fn persistent_voice_is_not_cache_and_failed_regeneration_preserves_old_wav() {
    let temp = Temp::new();
    let storage = temp.storage();
    let project = Id::new_v4();
    let bytes = wav();
    let a = voicevox::persist(&storage, project, &bytes, &VoicevoxSettings::default()).unwrap();
    let path = storage.resolve(project, &a);
    assert!(path.starts_with(&storage.data));
    assert!(!path.starts_with(&storage.cache));
    assert!(
        voicevox::persist(&storage, project, b"bad wav", &VoicevoxSettings::default()).is_err()
    );
    assert!(
        voicevox::persist(
            &storage,
            project,
            &bytes[..bytes.len() - 2],
            &VoicevoxSettings::default()
        )
        .is_err()
    );
    assert_eq!(fs::read(&path).unwrap(), bytes);
    let AssetLocation::Voice(id) = a.location else {
        panic!("not a generated asset")
    };
    assert!(storage.save_voice(project, id, b"overwrite").is_err());
    assert_eq!(fs::read(path).unwrap(), bytes);
    let mut p = Project {
        project_uuid: project,
        ..Default::default()
    };
    p.assets.push(a);
    let json = serde_json::to_string(&p).unwrap();
    assert!(!json.contains(".wav"));
    assert!(!json.contains("RIFF"));
}
#[test]
fn data_cache_overlap_and_git_storage_prevent_voice_writes() {
    let temp = Temp::new();
    let mut storage = temp.storage();
    storage.cache = storage.data.join("cache");
    assert!(
        storage
            .save_voice(Id::new_v4(), Id::new_v4(), &wav())
            .is_err()
    );
    assert!(!storage.data.exists());
    fs::create_dir_all(temp.path(".git")).unwrap();
    let storage = temp.storage();
    assert!(
        storage
            .save_voice(Id::new_v4(), Id::new_v4(), &wav())
            .is_err()
    );
    assert!(!storage.data.exists());
}
#[test]
fn language_falls_back_and_themes_are_serializable() {
    let mut i18n = I18n::default();
    assert_eq!(i18n.text("file"), "ファイル");
    i18n.language = "en-US".into();
    assert_eq!(i18n.text("file"), "File");
    assert!(i18n.add("fr-FR", "file = Fichier"));
    i18n.language = "fr-FR".into();
    assert_eq!(i18n.text("save"), "Save");
    assert_eq!(i18n.text("unknown-key"), "unknown-key");
    let settings = Settings {
        theme: Theme::Light,
        ..Default::default()
    };
    assert_eq!(
        serde_json::from_slice::<Settings>(&serde_json::to_vec(&settings).unwrap())
            .unwrap()
            .theme,
        Theme::Light
    );
    let context = eframe::egui::Context::default();
    context.set_theme(eframe::egui::ThemePreference::Light);
    assert_eq!(
        context.options(|o| o.theme_preference),
        eframe::egui::ThemePreference::Light
    );
}
#[test]
fn corrupted_settings_start_with_defaults_without_overwriting_original() {
    let temp = Temp::new();
    let storage = temp.storage();
    fs::create_dir_all(&storage.config).unwrap();
    let path = storage.config.join("settings.json");
    fs::write(&path, b"broken").unwrap();
    let (settings, error) = Settings::load(&storage);
    assert!(error.is_some());
    assert_eq!(settings.voicevox_url, "http://127.0.0.1:50021");
    assert_eq!(fs::read(&path).unwrap(), b"broken");
}
#[test]
fn ffprobe_parses_vfr_rotation_audio_and_japanese_paths() {
    let bytes = br#"{"streams":[{"index":0,"codec_type":"video","codec_name":"h264","width":1920,"height":1080,"avg_frame_rate":"30000/1001","side_data_list":[{"rotation":90}]},{"index":1,"codec_type":"audio","sample_rate":"48000"}],"format":{"duration":"3.5","start_time":"0.02","format_name":"mov,mp4"}}"#;
    let a = ffprobe::parse(bytes, Path::new("/tmp/日本語;test.mp4")).unwrap();
    assert_eq!(a.kind, MediaKind::Video);
    assert!(a.has_audio);
    assert_eq!(a.rotation, 90);
    assert_eq!(a.sample_rate, Some(48000));
    assert_eq!(a.frame_rate.unwrap().denominator, 1001);
    assert!(ffprobe::parse(b"{}", Path::new("/tmp/bad")).is_err());
}
#[test]
fn ffmpeg_paths_never_enter_filter_graph_and_sample_delay_is_exact() {
    let temp = Temp::new();
    let a = asset(&temp, "日本語';$(touch INJECTED).wav", MediaKind::Audio);
    let mut p = Project {
        frame_rate: FrameRate {
            numerator: 30000,
            denominator: 1001,
        },
        ..Default::default()
    };
    p.tracks[0].clips.push(clip(&a, 1, 30));
    p.assets.push(a.clone());
    let compiled = RenderPlan::new(&p, &temp.storage())
        .unwrap()
        .compile(None)
        .unwrap();
    let path = match a.location {
        AssetLocation::External(p) => p,
        _ => unreachable!(),
    };
    assert!(compiled.args.contains(&path.into_os_string()));
    let index = compiled
        .args
        .iter()
        .position(|a| a == "-filter_complex")
        .unwrap();
    let graph = compiled.args[index + 1].to_string_lossy();
    assert!(!graph.contains("INJECTED"));
    assert!(graph.contains("adelay=1601S"));
    assert!(!temp.path("INJECTED").exists());
}
#[test]
fn encoder_and_muxer_detection_uses_exact_names() {
    let caps = Capabilities {
        encoders: " V....D libx264 H264\n A..... aac AAC".into(),
        formats: " DE mov,mp4 Quicktime\n E webm WebM".into(),
        ..Default::default()
    };
    assert!(caps.supports_encoder("libx264"));
    assert!(!caps.supports_encoder("libx26"));
    assert!(caps.supports_muxer("mp4"));
    assert!(!caps.supports_muxer("avi"));
}
#[test]
fn voicevox_url_requires_explicit_remote_opt_in_and_disallows_redirect_urls() {
    for url in [
        "http://127.0.0.1:50021",
        "http://localhost:50021",
        "http://[::1]:50021",
    ] {
        assert!(voicevox::validate_url(url, false).is_ok(), "{url}");
    }
    assert!(voicevox::validate_url("http://example.com:50021", false).is_err());
    assert!(voicevox::validate_url("http://example.com:50021", true).is_ok());
    for url in [
        "file:///etc/passwd",
        "http://127.0.0.1:50021@evil.com",
        "http://127.0.0.1:50021/?next=evil",
        "http://127.0.0.1:50021/#evil",
    ] {
        assert!(voicevox::validate_url(url, true).is_err());
    }
}
#[test]
fn legacy_talk_and_streaming_talk_are_supported_but_singing_is_excluded() {
    for (value, expected) in [
        (None, true),
        (Some("talk"), true),
        (Some("streaming_talk"), true),
        (Some("sing"), false),
        (Some("frame_decode"), false),
    ] {
        let style = voicevox::Style {
            id: 0,
            name: "test".into(),
            style_type: value.map(str::to_string),
        };
        assert_eq!(style.is_talk(), expected);
    }
}
fn read_request(stream: &mut std::net::TcpStream) -> String {
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .unwrap();
    let mut bytes = Vec::new();
    let mut byte = [0];
    while !bytes.ends_with(b"\r\n\r\n") {
        stream.read_exact(&mut byte).unwrap();
        bytes.push(byte[0]);
    }
    let header = String::from_utf8(bytes.clone()).unwrap();
    let length = header
        .lines()
        .find_map(|l| {
            l.to_ascii_lowercase()
                .strip_prefix("content-length:")
                .and_then(|s| s.trim().parse::<usize>().ok())
        })
        .unwrap_or(0);
    let mut body = vec![0; length];
    stream.read_exact(&mut body).unwrap();
    bytes.extend(body);
    String::from_utf8(bytes).unwrap()
}
#[test]
fn voicevox_http_contract_parameter_editing_and_failure_protection() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let audio = wav();
    let server = thread::spawn(move || {
        let mut requests = Vec::new();
        for i in 0..6 {
            let (mut stream, _) = listener.accept().unwrap();
            let request = read_request(&mut stream);
            requests.push(request);
            let (status, content_type, body) = match i {
                0 => ("200 OK", "application/json", br#""0.test""#.to_vec()),
                1 => ("200 OK", "application/json", br#"[{"name":"Speaker","speaker_uuid":"uuid","styles":[{"id":3,"name":"Normal","type":"talk"},{"id":4,"name":"Song","type":"sing"}]}]"#.to_vec()),
                2 | 4 => ("200 OK", "application/json", br#"{"accent_phrases":[],"speedScale":1,"outputSamplingRate":24000}"#.to_vec()),
                3 => ("200 OK", "audio/wav", audio.clone()),
                _ => ("500 Internal Server Error", "application/json", b"{}".to_vec()),
            };
            write!(stream, "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).unwrap();
            stream.write_all(&body).unwrap();
        }
        requests
    });
    let client = Client::new(&url, false).unwrap();
    let connection = client.connect().unwrap();
    assert_eq!(connection.version, "0.test");
    assert_eq!(connection.speakers[0].styles.len(), 1);
    let settings = VoicevoxSettings {
        speaker: 3,
        speed: 1.3,
        ..Default::default()
    };
    let (bytes, duration, sample_rate) = client.synthesize("こんにちは & test", &settings).unwrap();
    assert_eq!(duration, 0.1);
    assert_eq!(sample_rate, 48000);
    let temp = Temp::new();
    let storage = temp.storage();
    let project = Id::new_v4();
    let a = voicevox::persist(&storage, project, &bytes, &settings).unwrap();
    assert!(client.synthesize("失敗", &settings).is_err());
    assert_eq!(fs::read(storage.resolve(project, &a)).unwrap(), bytes);
    let requests = server.join().unwrap();
    assert!(requests[2].starts_with("POST /audio_query?"));
    assert!(requests[2].contains("speaker=3"));
    assert!(requests[2].contains("%E3"));
    let body: serde_json::Value =
        serde_json::from_str(requests[3].split_once("\r\n\r\n").unwrap().1).unwrap();
    assert_eq!(body["speedScale"], 1.3);
    assert_eq!(body["outputSamplingRate"], 48000);
    assert!(body["accent_phrases"].is_array());
}

#[test]
fn automatic_placement_respects_layer_flags_limits_and_timeline_bounds() {
    let mut project = Project::default();
    project.tracks[0].locked = true;
    project.tracks[1].enabled = false;
    project.tracks[2].video_visible = false;
    project.tracks[3].muted = true;
    assert_eq!(
        commands::placement_track(&mut project, 0, 0, 30).unwrap(),
        4
    );
    let before = project.clone();
    assert_eq!(
        commands::placement_track(&mut project, 0, MAX_FRAME, 1).unwrap_err(),
        "clip-outside-timeline"
    );
    assert_eq!(project, before);
    for index in project.tracks.len()..256 {
        project.tracks.push(Track::new(index + 1));
    }
    project.tracks[255].locked = true;
    let before = project.clone();
    let mut history = History::default();
    assert_eq!(
        history
            .edit(&mut project, |p| commands::placement_track(p, 255, 0, 30))
            .unwrap_err(),
        "no-free-layer"
    );
    assert_eq!(project, before);
}

#[test]
fn clip_edge_resize_bounds_preserve_sources_neighbors_and_undo() {
    use commands::ClipEdge::{End, Start};
    let temp = Temp::new();
    for kind in [MediaKind::Video, MediaKind::Audio, MediaKind::Image] {
        let a = asset(&temp, "resize", kind);
        let mut p = Project::default();
        let mut c = clip(&a, 60, 60);
        c.source_offset = 30;
        let id = c.id;
        p.assets.push(a.clone());
        p.tracks[0].clips.push(c);
        let left = commands::resized_clip(&p, id, Start, 0).unwrap();
        assert_eq!(
            left.timeline_start,
            if kind == MediaKind::Image { 0 } else { 30 }
        );
        assert_eq!(left.source_offset, 0);
        assert_eq!(left.end(), 120);
        let right = commands::resized_clip(&p, id, End, 1000).unwrap();
        assert_eq!(
            right.end(),
            if kind == MediaKind::Image { 1000 } else { 330 }
        );
        p.tracks[0].clips.push(clip(&a, 20, 20));
        p.tracks[0].clips.push(clip(&a, 160, 20));
        assert_eq!(
            commands::resized_clip(&p, id, Start, 0)
                .unwrap()
                .timeline_start,
            40
        );
        assert_eq!(commands::resized_clip(&p, id, End, 200).unwrap().end(), 160);
        assert_eq!(
            commands::resized_clip(&p, id, Start, 999).unwrap().duration,
            1
        );
        assert_eq!(commands::resized_clip(&p, id, End, 0).unwrap().duration, 1);
        let original = p.clone();
        let mut history = History::default();
        history
            .edit(&mut p, |p| commands::resize_clip(p, id, Start, 90))
            .unwrap();
        assert_eq!(
            (
                p.clip(id).unwrap().source_offset,
                p.clip(id).unwrap().duration
            ),
            (60, 30)
        );
        history.undo(&mut p);
        assert_eq!(p, original);
        history.redo(&mut p);
        assert_eq!(p.clip(id).unwrap().timeline_start, 90);
        p.tracks[0].locked = true;
        assert_eq!(
            commands::resize_clip(&mut p, id, End, 150).unwrap_err(),
            "track-locked"
        );
    }
    for source in [
        ClipSource::Sprite {
            settings_id: Id::new_v4(),
        },
        ClipSource::Text {
            settings: TextSettings {
                text: "title".into(),
                ..Default::default()
            },
        },
    ] {
        let mut p = Project::default();
        let a = asset(&temp, "unused", MediaKind::Image);
        let mut c = clip(&a, 30, 30);
        c.source = source;
        let id = c.id;
        p.tracks[0].clips.push(c);
        commands::resize_clip(&mut p, id, End, 600).unwrap();
        assert_eq!(p.clip(id).unwrap().duration, 570);
        commands::resize_clip(&mut p, id, Start, 0).unwrap();
        assert_eq!(p.clip(id).unwrap().duration, 600);
        p.validate().unwrap();
    }
}
