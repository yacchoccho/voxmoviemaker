use std::{
    fs,
    path::PathBuf,
    sync::{Arc, atomic::AtomicBool},
};
use voxmoviemaker::{
    assets::Storage,
    media::{cache, export, ffmpeg, render_plan::RenderPlan, text},
    project::{
        commands::{self, History},
        persistence, portability, *,
    },
    settings::{ExportFormat, Settings},
};
struct Fixture {
    root: PathBuf,
    storage: Storage,
    settings: Settings,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("vmm-phase5-{}", Id::new_v4()));
        fs::create_dir(&root).unwrap();
        Self {
            storage: Storage {
                data: root.join("data"),
                cache: root.join("cache"),
                config: root.join("config"),
            },
            root,
            settings: Settings::default(),
        }
    }
    fn image(
        &self,
        p: &mut Project,
        track: usize,
        color: [u8; 4],
        start: u64,
        duration: u64,
    ) -> Id {
        let id = Id::new_v4();
        let path = self.root.join(format!("{id}.png"));
        image::RgbaImage::from_pixel(160, 90, image::Rgba(color))
            .save(&path)
            .unwrap();
        p.assets.push(MediaAsset {
            id,
            name: "synthetic image".into(),
            location: AssetLocation::External(path),
            kind: MediaKind::Image,
            duration_seconds: 0.0,
            width: 160,
            height: 90,
            has_audio: false,
            sample_rate: None,
            frame_rate: None,
            codec: "png".into(),
            start_time: 0.0,
            rotation: 0,
            video_stream: Some(0),
            audio_stream: None,
        });
        let clip = Clip {
            character: None,
            id: Id::new_v4(),
            source: ClipSource::Media { asset_id: id },
            timeline_start: start,
            duration,
            source_offset: 0,
            volume: 1.0,
            muted: false,
            effects: Default::default(),
        };
        let id = clip.id;
        p.tracks[track].clips.push(clip);
        id
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
fn text_clip(value: &str) -> Clip {
    Clip {
        character: None,
        id: Id::new_v4(),
        source: ClipSource::Text {
            settings: TextSettings {
                text: value.into(),
                size: 48.0,
                anchor: TextAnchor::Center,
                ..Default::default()
            },
        },
        timeline_start: 0,
        duration: 60,
        source_offset: 0,
        volume: 1.0,
        muted: false,
        effects: Default::default(),
    }
}

#[test]
#[ignore = "requires user-installed FFmpeg/ffprobe"]
fn actual_interactive_preview_planes_preserve_alpha_order_and_transforms() {
    use voxmoviemaker::{media::preview, tasks::PreviewRequest};
    let f = Fixture::new();
    let mut p = Project {
        width: 320,
        height: 180,
        ..Default::default()
    };
    f.image(&mut p, 0, [255, 0, 0, 255], 0, 30);
    let selected = f.image(&mut p, 1, [0, 255, 0, 255], 0, 30);
    let front = f.image(&mut p, 2, [0, 0, 255, 255], 0, 30);
    p.clip_mut(selected).unwrap().effects = ClipEffects {
        scale: 0.5,
        x: 30.0,
        opacity: 0.6,
        ..Default::default()
    };
    p.clip_mut(front).unwrap().effects = ClipEffects {
        scale: 0.25,
        x: -80.0,
        ..Default::default()
    };
    let frame = preview::render(
        PreviewRequest {
            project: p.project_uuid,
            selected: Some(selected),
            plan: RenderPlan::new(&p, &f.storage).unwrap(),
            storage: f.storage.clone(),
            ffmpeg: f.settings.ffmpeg.clone(),
            frame: 5,
            width: 320,
            limit_mb: 8,
        },
        &|| false,
    )
    .unwrap();
    let planes = frame.planes.unwrap();
    let center = planes.object.get_pixel(160, 90).0;
    assert!(center[1] > 240 && center[3].abs_diff(153) < 5, "{center:?}");
    assert_eq!(
        planes.front.get_pixel(300, 10)[3],
        0,
        "front plane must be transparent"
    );
    assert!(planes.front.get_pixel(80, 90)[2] > 240);
    assert_eq!(
        frame.objects.iter().map(|o| o.id).collect::<Vec<_>>(),
        p.tracks
            .iter()
            .flat_map(|t| t.clips.iter().map(|c| c.id))
            .collect::<Vec<_>>()
    );
    // Reuse the same three planes after moving/scaling only the selected object.
    for (scale, x) in [(0.5, 30.0), (0.75, 50.0)] {
        let effects = &mut p.clip_mut(selected).unwrap().effects;
        effects.scale = scale;
        effects.x = x;
        let width = (320.0 * scale).round() as u32;
        let height = (180.0 * scale).round() as u32;
        let resized = image::imageops::resize(
            &planes.object,
            width,
            height,
            image::imageops::FilterType::Triangle,
        );
        let mut live = planes.behind.clone();
        image::imageops::overlay(
            &mut live,
            &resized,
            (320 - width) as i64 / 2 + x as i64,
            (180 - height) as i64 / 2,
        );
        image::imageops::overlay(&mut live, &planes.front, 0, 0);
        let rendered = cache::preview(
            &RenderPlan::new(&p, &f.storage).unwrap(),
            &f.storage,
            &f.settings.ffmpeg,
            5,
            320,
            8,
            || false,
        )
        .unwrap();
        for (px, py) in [(15, 15), (80, 90), (180, 90), (250, 90)] {
            for channel in 0..3 {
                assert!(
                    live.get_pixel(px, py)[channel].abs_diff(rendered.get_pixel(px, py)[channel])
                        < 12,
                    "scale={scale}, at {px},{py}: live={:?}, rendered={:?}",
                    live.get_pixel(px, py),
                    rendered.get_pixel(px, py)
                );
            }
        }
    }
}

#[test]
fn text_and_effects_save_roundtrip_and_v1_migration() {
    let f = Fixture::new();
    let mut p = Project::default();
    p.tracks[0].clips.push(text_clip("日本語\nText"));
    p.tracks[0].clips[0].effects = ClipEffects {
        x: 25.0,
        scale: 0.75,
        opacity: 0.5,
        fade_in: 15,
        transition: TransitionKind::SlideLeft,
        transition_frames: 15,
        ..Default::default()
    };
    let path = f.root.join("test.vmm.json");
    persistence::save(&path, &p).unwrap();
    assert_eq!(persistence::load(&path).unwrap(), p);
    let mut old = Project::default();
    f.image(&mut old, 0, [255, 0, 0, 255], 0, 60);
    let mut legacy = serde_json::to_value(&old).unwrap();
    legacy["schema_version"] = 1.into();
    legacy["tracks"][0]["clips"][0]
        .as_object_mut()
        .unwrap()
        .remove("effects");
    fs::write(&path, serde_json::to_vec(&legacy).unwrap()).unwrap();
    assert_eq!(
        persistence::load(&path).unwrap().schema_version,
        SCHEMA_VERSION
    );
    assert_eq!(
        persistence::load(&path).unwrap().tracks[0].clips[0].effects,
        ClipEffects::default()
    );
}
#[test]
fn invalid_effects_and_text_are_rejected_transactionally() {
    let mut p = Project::default();
    let c = text_clip("safe");
    let id = c.id;
    p.tracks[0].clips.push(c);
    let before = p.clone();
    let mut history = History::default();
    assert!(
        history
            .edit(&mut p, |p| {
                p.clip_mut(id).unwrap().effects.scale = f64::NAN;
                Ok(())
            })
            .is_err()
    );
    assert_eq!(p, before);
    assert!(
        history
            .edit(&mut p, |p| {
                p.clip_mut(id).unwrap().effects.fade_in = 61;
                Ok(())
            })
            .is_err()
    );
    assert!(
        history
            .edit(&mut p, |p| {
                p.clip_mut(id).unwrap().source = ClipSource::Text {
                    settings: TextSettings {
                        text: "a".repeat(4097),
                        ..Default::default()
                    },
                };
                Ok(())
            })
            .is_err()
    );
    assert_eq!(p, before);
}
#[test]
fn split_text_clamps_effects_and_preserves_undo() {
    let f = Fixture::new();
    let mut p = Project::default();
    f.image(&mut p, 0, [255, 0, 0, 255], 0, 30);
    let mut c = text_clip("caption");
    c.timeline_start = 30;
    c.effects.fade_in = 30;
    let id = c.id;
    p.tracks[0].clips.push(c);
    let before = p.clone();
    let mut h = History::default();
    h.edit(&mut p, |p| commands::split(p, id, 40)).unwrap();
    assert_eq!(p.clip(id).unwrap().effects.fade_in, 10);
    assert_eq!(p.tracks[0].clips.len(), 3);
    p.validate().unwrap();
    h.undo(&mut p);
    assert_eq!(p, before);
}
#[test]
fn crossfade_keeps_generic_tracks_sources_and_layer_order() {
    let f = Fixture::new();
    let mut p = Project::default();
    let a = f.image(&mut p, 0, [255, 0, 0, 255], 0, 60);
    let b = f.image(&mut p, 0, [0, 0, 255, 255], 60, 60);
    let before = p.clone();
    let mut h = History::default();
    h.edit(&mut p, |p| commands::crossfade_next(p, a, 15).map(|_| ()))
        .unwrap();
    assert_eq!(p.track_of(a), Some(0));
    assert_eq!(p.track_of(b), Some(1));
    assert_eq!(p.clip(b).unwrap().timeline_start, 45);
    assert_eq!(
        p.clip(b).unwrap().effects.transition,
        TransitionKind::Dissolve
    );
    assert_eq!(p.clip(a).unwrap().effects.audio_fade_out, 15);
    assert_eq!(
        RenderPlan::new(&p, &f.storage).unwrap().visual_order(50),
        vec![a, b]
    );
    h.undo(&mut p);
    assert_eq!(p, before);
}
#[test]
fn font_rasterization_handles_japanese_transparency_and_hostile_text_as_data() {
    let style = TextSettings {
        text: "日本語 ';[out] $(touch INJECTED)\n字幕".into(),
        ..Default::default()
    };
    let image = text::rasterize(&style, 640, 360, 640).unwrap();
    assert!(image.pixels().any(|p| p[3] > 0));
    assert_eq!(image.get_pixel(0, 0)[3], 0);
    assert!(!PathBuf::from("INJECTED").exists());
}
#[test]
fn text_temporary_inputs_live_until_compile_is_dropped() {
    let f = Fixture::new();
    let mut p = Project::default();
    p.tracks[0].clips.push(text_clip("Text"));
    let plan = RenderPlan::new(&p, &f.storage).unwrap();
    let compiled = plan.compile(Some((0, 320))).unwrap();
    let paths: Vec<_> = fs::read_dir(f.storage.cache.join("text"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert_eq!(paths.len(), 1);
    assert!(paths[0].is_file());
    drop(compiled);
    assert!(!paths[0].exists());
}
#[test]
fn portable_export_copies_only_used_assets_and_keeps_sources_immutable() {
    let f = Fixture::new();
    let mut p = Project::default();
    f.image(&mut p, 0, [255, 0, 0, 255], 0, 60);
    f.image(&mut p, 1, [0, 0, 255, 255], 0, 60);
    let unused = p.tracks[1].clips.remove(0).source.asset_id().unwrap();
    p.tracks[2].clips.push(text_clip("字幕"));
    let source = f.storage.resolve(p.project_uuid, &p.assets[0]);
    let bytes = fs::read(&source).unwrap();
    let target = f.root.join("portable");
    portability::export(&p, &f.storage, &target, || false, |_| {}).unwrap();
    let loaded = persistence::load(&target.join("project.vmm.json")).unwrap();
    assert_eq!(loaded.assets.len(), 1);
    assert!(loaded.asset(unused).is_err());
    let AssetLocation::External(path) = &loaded.assets[0].location else {
        panic!()
    };
    assert!(path.starts_with(&target));
    assert_eq!(fs::read(path).unwrap(), bytes);
    assert_eq!(fs::read(source).unwrap(), bytes);
    assert_eq!(p.assets.len(), 2);
}
#[test]
fn portable_cancel_git_and_existing_destination_are_protected() {
    let f = Fixture::new();
    let mut p = Project::default();
    f.image(&mut p, 0, [255, 0, 0, 255], 0, 60);
    let target = f.root.join("portable");
    assert_eq!(
        portability::export(&p, &f.storage, &target, || true, |_| {}).unwrap_err(),
        "cancelled"
    );
    assert!(!target.exists());
    fs::create_dir(&target).unwrap();
    fs::write(target.join("keep"), b"keep").unwrap();
    assert_eq!(
        portability::export(&p, &f.storage, &target, || false, |_| {}).unwrap_err(),
        "output-exists"
    );
    assert_eq!(fs::read(target.join("keep")).unwrap(), b"keep");
    let repo = f.root.join("repository");
    fs::create_dir_all(repo.join(".git")).unwrap();
    assert_eq!(
        portability::export(&p, &f.storage, &repo.join("export"), || false, |_| {}).unwrap_err(),
        "package-in-git"
    );
    assert!(!fs::read_dir(&f.root).unwrap().any(|e| {
        e.unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".voxmoviemaker-package-")
    }));
}

#[test]
fn three_bundled_languages_have_complete_keys_and_parse_as_fluent() {
    use std::collections::BTreeSet;
    let sources = [
        ("en-US", include_str!("../locales/en-US/main.ftl")),
        ("ja-JP", include_str!("../locales/ja-JP/main.ftl")),
        ("zh-CN", include_str!("../locales/zh-CN/main.ftl")),
    ];
    let keys = |s: &str| -> BTreeSet<String> {
        s.lines()
            .filter_map(|line| line.split_once(" = ").map(|(key, _)| key.to_string()))
            .collect()
    };
    let expected = keys(sources[0].1);
    let mut i18n = voxmoviemaker::i18n::I18n::default();
    for (language, source) in sources {
        assert_eq!(keys(source), expected, "{language}");
        assert!(i18n.add(language, source));
        i18n.language = language.into();
        assert_ne!(i18n.text("export-package"), "export-package");
    }
}

#[test]
fn portable_export_rejects_programs_and_fonts_disguised_as_media() {
    let f = Fixture::new();
    let mut p = Project::default();
    f.image(&mut p, 0, [255, 0, 0, 255], 0, 30);
    let source = f.storage.resolve(p.project_uuid, &p.assets[0]);
    for bytes in [
        b"\x7fELFprogram".as_slice(),
        b"MZprogram",
        b"ttcffont",
        b"PK\x03\x04model",
    ] {
        fs::write(&source, bytes).unwrap();
        let target = f.root.join("portable");
        assert_eq!(
            portability::export(&p, &f.storage, &target, || false, |_| {}).unwrap_err(),
            "unsupported-media"
        );
        assert!(!target.exists());
    }
}

#[test]
fn portable_voice_project_reloads_after_moving_without_original_wav() {
    let f = Fixture::new();
    let mut project = Project::default();
    let mut cursor = std::io::Cursor::new(Vec::new());
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
        for _ in 0..48000 {
            writer.write_sample(1000i16).unwrap();
        }
        writer.finalize().unwrap();
    }
    let voice = voxmoviemaker::voicevox::persist(
        &f.storage,
        project.project_uuid,
        cursor.get_ref(),
        &Default::default(),
    )
    .unwrap();
    let original = f.storage.resolve(project.project_uuid, &voice);
    project.tracks[0].clips.push(Clip {
        character: None,
        id: Id::new_v4(),
        source: ClipSource::Voicevox {
            asset_id: voice.id,
            text: "private test dialogue".into(),
            settings: Default::default(),
            subtitle: None,
        },
        timeline_start: 0,
        duration: 30,
        source_offset: 0,
        volume: 1.0,
        muted: false,
        effects: Default::default(),
    });
    project.assets.push(voice);
    let target = f.root.join("package");
    portability::export(&project, &f.storage, &target, || false, |_| {}).unwrap();
    fs::remove_file(original).unwrap();
    let moved = f.root.join("moved");
    fs::rename(target, &moved).unwrap();
    let loaded = persistence::load(&moved.join("project.vmm.json")).unwrap();
    let AssetLocation::External(path) = &loaded.assets[0].location else {
        panic!()
    };
    assert!(path.starts_with(&moved));
    assert_eq!(fs::read(path).unwrap(), *cursor.get_ref());
    assert!(
        matches!(&loaded.tracks[0].clips[0].source,ClipSource::Voicevox{text,..} if text=="private test dialogue")
    );
    assert_eq!(fs::read_dir(moved.join("media")).unwrap().count(), 1);
}

#[test]
#[ignore = "requires user-installed FFmpeg/ffprobe with libx264/aac"]
fn actual_transform_opacity_fades_slide_and_text_match_preview_export() {
    for fps in [
        FrameRate::default(),
        FrameRate {
            numerator: 30000,
            denominator: 1001,
        },
    ] {
        let f = Fixture::new();
        let mut p = Project {
            width: 320,
            height: 180,
            frame_rate: fps,
            ..Default::default()
        };
        f.image(&mut p, 0, [255, 0, 0, 255], 0, 60);
        let front = f.image(&mut p, 1, [0, 0, 255, 255], 0, 60);
        p.clip_mut(front).unwrap().effects = ClipEffects {
            scale: 0.5,
            x: 40.0,
            opacity: 0.5,
            fade_in: 30,
            fade_out: 30,
            ..Default::default()
        };
        let plan = RenderPlan::new(&p, &f.storage).unwrap();
        let frame =
            cache::preview(&plan, &f.storage, &f.settings.ffmpeg, 15, 320, 8, || false).unwrap();
        let center = frame.get_pixel(200, 90);
        assert!(
            (center[0] as i32 - 191).abs() < 10 && (center[2] as i32 - 64).abs() < 10,
            "{center:?}"
        );
        assert!(frame.get_pixel(20, 90)[0] > 240);
        let caps = ffmpeg::detect(&f.settings).unwrap();
        let output = f.root.join("effects.mp4");
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
        let decoded = ffmpeg::run(
            std::process::Command::new(&f.settings.ffmpeg)
                .args(["-ss", &fps.timestamp(15), "-i"])
                .arg(&output)
                .args([
                    "-frames:v",
                    "1",
                    "-f",
                    "image2pipe",
                    "-c:v",
                    "png",
                    "pipe:1",
                ]),
            std::time::Duration::from_secs(10),
            || false,
        )
        .unwrap();
        let exported = image::load_from_memory(&decoded.stdout).unwrap().to_rgba8();
        for channel in 0..3 {
            assert!(
                (exported.get_pixel(200, 90)[channel] as i32 - center[channel] as i32).abs() < 15
            );
        }
        p.clip_mut(front).unwrap().effects = ClipEffects {
            transition: TransitionKind::SlideLeft,
            transition_frames: 30,
            ..Default::default()
        };
        let slide = RenderPlan::new(&p, &f.storage).unwrap();
        let first =
            cache::preview(&slide, &f.storage, &f.settings.ffmpeg, 0, 320, 8, || false).unwrap();
        assert!(first.get_pixel(160, 90)[0] > 240);
        let middle =
            cache::preview(&slide, &f.storage, &f.settings.ffmpeg, 15, 320, 8, || false).unwrap();
        assert!(middle.get_pixel(40, 90)[0] > 240);
        assert!(middle.get_pixel(280, 90)[2] > 240);
        p.tracks[2].clips.push(text_clip("字幕"));
        let text_plan = RenderPlan::new(&p, &f.storage).unwrap();
        let text_frame = cache::preview(
            &text_plan,
            &f.storage,
            &f.settings.ffmpeg,
            40,
            320,
            8,
            || false,
        )
        .unwrap();
        assert!(
            text_frame
                .pixels()
                .any(|pixel| pixel[0] > 200 && pixel[1] > 200 && pixel[2] > 200)
        );
        let target = f.root.join("subtitle.mp4");
        export::export(
            &text_plan,
            &f.settings.ffmpeg,
            &caps,
            ExportFormat::Mp4,
            &target,
            Arc::new(AtomicBool::new(false)),
            |_| {},
        )
        .unwrap();
        let decoded = ffmpeg::run(
            std::process::Command::new(&f.settings.ffmpeg)
                .args(["-ss", &fps.timestamp(40), "-i"])
                .arg(target)
                .args([
                    "-frames:v",
                    "1",
                    "-f",
                    "image2pipe",
                    "-c:v",
                    "png",
                    "pipe:1",
                ]),
            std::time::Duration::from_secs(10),
            || false,
        )
        .unwrap();
        let image = image::load_from_memory(&decoded.stdout).unwrap().to_rgba8();
        assert!(
            image
                .pixels()
                .any(|p| p[0] > 200 && p[1] > 200 && p[2] > 200)
        );
        let request = voxmoviemaker::media::playback::ProxyRequest {
            plan: text_plan,
            storage: f.storage.clone(),
            ffmpeg: f.settings.ffmpeg.clone(),
            width: 320,
            limit_mb: 8,
            progress: Arc::new(std::sync::atomic::AtomicU32::new(0)),
        };
        let proxy = voxmoviemaker::media::playback::prepare(&request, &|| false, || {}).unwrap();
        assert!(proxy.path.is_file());
        assert_eq!(
            fs::read_dir(f.storage.cache.join("text")).unwrap().count(),
            0
        );
    }
}

#[test]
#[ignore = "requires user-installed FFmpeg/ffprobe with mpeg4 and PCM"]
fn actual_audio_crossfade_keeps_mix_level_through_overlap() {
    let f = Fixture::new();
    let path = f.root.join("tone.wav");
    let mut writer = hound::WavWriter::create(
        &path,
        hound::WavSpec {
            channels: 1,
            sample_rate: 48000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        },
    )
    .unwrap();
    for sample in 0..96000 {
        writer
            .write_sample(
                ((sample as f64 * std::f64::consts::TAU * 440.0 / 48000.0).sin() * 4000.0) as i16,
            )
            .unwrap();
    }
    writer.finalize().unwrap();
    let asset = voxmoviemaker::media::ffprobe::probe(&f.settings.ffprobe, &path).unwrap();
    let mut project = Project::default();
    let mut first = text_clip("");
    first.source = ClipSource::Media { asset_id: asset.id };
    let id = first.id;
    let mut second = first.clone();
    second.id = Id::new_v4();
    second.timeline_start = 60;
    project.assets.push(asset);
    project.tracks[0].clips = vec![first, second];
    commands::crossfade_next(&mut project, id, 15).unwrap();
    let request = voxmoviemaker::media::playback::ProxyRequest {
        plan: RenderPlan::new(&project, &f.storage).unwrap(),
        storage: f.storage.clone(),
        ffmpeg: f.settings.ffmpeg.clone(),
        width: 160,
        limit_mb: 8,
        progress: Arc::new(std::sync::atomic::AtomicU32::new(0)),
    };
    let proxy = voxmoviemaker::media::playback::prepare(&request, &|| false, || {}).unwrap();
    let output = ffmpeg::run(
        std::process::Command::new(&f.settings.ffmpeg)
            .args(["-i"])
            .arg(proxy.path)
            .args([
                "-ss", "1.6", "-t", "0.2", "-vn", "-ac", "1", "-f", "f32le", "pipe:1",
            ]),
        std::time::Duration::from_secs(10),
        || false,
    )
    .unwrap();
    let samples: Vec<_> = output
        .stdout
        .as_chunks::<4>()
        .0
        .iter()
        .copied()
        .map(f32::from_le_bytes)
        .collect();
    let rms = (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt();
    let expected = 4000.0_f32 / 32768.0 / 2.0_f32.sqrt();
    assert!((rms - expected).abs() < 0.003, "{rms} vs {expected}");
}

fn combined_voice(f: &Fixture, project: &mut Project) -> Id {
    let path = f.root.join("voice.wav");
    let mut wav = hound::WavWriter::create(
        &path,
        hound::WavSpec {
            channels: 1,
            sample_rate: 48000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        },
    )
    .unwrap();
    for sample in 0..96000 {
        wav.write_sample(
            ((sample as f64 * std::f64::consts::TAU * 440.0 / 48000.0).sin() * 4000.0) as i16,
        )
        .unwrap();
    }
    wav.finalize().unwrap();
    let asset = MediaAsset {
        id: Id::new_v4(),
        name: "voice".into(),
        location: AssetLocation::External(path),
        kind: MediaKind::Audio,
        duration_seconds: 2.0,
        width: 0,
        height: 0,
        has_audio: true,
        sample_rate: Some(48000),
        frame_rate: None,
        codec: "pcm_s16le".into(),
        start_time: 0.0,
        rotation: 0,
        video_stream: None,
        audio_stream: Some(0),
    };
    let clip = Clip {
        character: None,
        id: Id::new_v4(),
        source: ClipSource::Voicevox {
            asset_id: asset.id,
            text: "音声と字幕".into(),
            settings: Default::default(),
            subtitle: Some(TextSettings {
                text: "音声と字幕".into(),
                size: 40.0,
                ..Default::default()
            }),
        },
        timeline_start: 30,
        duration: 60,
        source_offset: 0,
        volume: 1.0,
        muted: false,
        effects: Default::default(),
    };
    let id = clip.id;
    project.assets.push(asset);
    project.tracks[1].clips.push(clip);
    id
}

#[test]
fn combined_voice_persists_moves_splits_and_uses_normal_layer_visibility() {
    let f = Fixture::new();
    let mut p = Project::default();
    let background = f.image(&mut p, 0, [255, 0, 0, 255], 0, 120);
    let voice = combined_voice(&f, &mut p);
    let plan = RenderPlan::new(&p, &f.storage).unwrap();
    assert_eq!(plan.visual_order(45), vec![background, voice]);
    assert_eq!(plan.visual_order(0), vec![background]);
    assert_eq!(plan.clips.iter().filter(|c| c.audio).count(), 1);
    p.tracks[1].video_visible = false;
    let plan = RenderPlan::new(&p, &f.storage).unwrap();
    assert_eq!(plan.visual_order(45), vec![background]);
    assert!(plan.clips.iter().any(|c| c.id == voice && c.audio));
    p.tracks[1].video_visible = true;
    p.tracks[1].muted = true;
    let plan = RenderPlan::new(&p, &f.storage).unwrap();
    assert_eq!(plan.visual_order(45), vec![background, voice]);
    assert!(!plan.clips.iter().any(|c| c.audio));
    p.tracks[1].muted = false;
    let path = f.root.join("combined.vmm.json");
    persistence::save(&path, &p).unwrap();
    assert_eq!(persistence::load(&path).unwrap(), p);
    let before = p.clone();
    let mut history = History::default();
    history
        .edit(&mut p, |p| commands::move_clip(p, voice, 3, 0))
        .unwrap();
    history
        .edit(&mut p, |p| commands::split(p, voice, 30))
        .unwrap();
    assert_eq!(p.tracks[3].clips.len(), 2);
    assert_eq!(p.tracks[3].clips[1].source_offset, 30);
    for clip in &p.tracks[3].clips {
        let plan = RenderPlan::new(&p, &f.storage).unwrap();
        let parts: Vec<_> = plan.clips.iter().filter(|c| c.id == clip.id).collect();
        assert_eq!(parts.len(), 2);
        assert!(
            parts
                .iter()
                .all(|c| c.start == clip.timeline_start && c.duration == clip.duration)
        );
    }
    history.undo(&mut p);
    history.undo(&mut p);
    assert_eq!(p, before);
    let mut old = serde_json::to_value(&p).unwrap();
    old["schema_version"] = 2.into();
    old["tracks"][1]["clips"][0]["source"]["Voicevox"]
        .as_object_mut()
        .unwrap()
        .remove("subtitle");
    fs::write(&path, serde_json::to_vec(&old).unwrap()).unwrap();
    let loaded = persistence::load(&path).unwrap();
    assert_eq!(loaded.schema_version, SCHEMA_VERSION);
    assert!(matches!(
        loaded.clip(voice).unwrap().source,
        ClipSource::Voicevox { subtitle: None, .. }
    ));
}

#[test]
#[ignore = "requires user-installed FFmpeg/ffprobe with libx264/aac"]
fn actual_combined_voice_subtitle_matches_preview_export_and_audio_timing() {
    let f = Fixture::new();
    let mut p = Project {
        width: 320,
        height: 180,
        ..Default::default()
    };
    f.image(&mut p, 0, [255, 0, 0, 255], 0, 120);
    combined_voice(&f, &mut p);
    let plan = RenderPlan::new(&p, &f.storage).unwrap();
    let before =
        cache::preview(&plan, &f.storage, &f.settings.ffmpeg, 15, 320, 8, || false).unwrap();
    assert!(!before.pixels().any(|v| v[1] > 200 && v[2] > 200));
    let preview =
        cache::preview(&plan, &f.storage, &f.settings.ffmpeg, 45, 320, 8, || false).unwrap();
    assert!(
        preview
            .pixels()
            .any(|v| v[0] > 200 && v[1] > 200 && v[2] > 200)
    );
    let output = f.root.join("voice-subtitle.mp4");
    export::export(
        &plan,
        &f.settings.ffmpeg,
        &ffmpeg::detect(&f.settings).unwrap(),
        ExportFormat::Mp4,
        &output,
        Arc::new(AtomicBool::new(false)),
        |_| {},
    )
    .unwrap();
    let frame = ffmpeg::run(
        std::process::Command::new(&f.settings.ffmpeg)
            .args(["-ss", "1.5", "-i"])
            .arg(&output)
            .args([
                "-frames:v",
                "1",
                "-f",
                "image2pipe",
                "-c:v",
                "png",
                "pipe:1",
            ]),
        std::time::Duration::from_secs(10),
        || false,
    )
    .unwrap();
    let exported = image::load_from_memory(&frame.stdout).unwrap().to_rgba8();
    let white: Vec<_> = preview
        .enumerate_pixels()
        .filter(|(_, _, v)| v[0] > 230 && v[1] > 230 && v[2] > 230)
        .collect();
    assert!(white.len() > 30);
    assert!(
        white
            .iter()
            .filter(|(x, y, _)| exported.get_pixel(*x, *y)[1] > 180)
            .count()
            * 10
            > white.len() * 9
    );
    for (time, silent) in [("0.3", true), ("1.5", false), ("3.3", true)] {
        let audio = ffmpeg::run(
            std::process::Command::new(&f.settings.ffmpeg)
                .args(["-ss", time, "-i"])
                .arg(&output)
                .args(["-t", "0.2", "-vn", "-ac", "1", "-f", "f32le", "pipe:1"]),
            std::time::Duration::from_secs(10),
            || false,
        )
        .unwrap();
        let samples: Vec<_> = audio
            .stdout
            .as_chunks::<4>()
            .0
            .iter()
            .copied()
            .map(f32::from_le_bytes)
            .collect();
        assert!(!samples.is_empty());
        let rms = (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt();
        if silent {
            assert!(rms < 0.005, "{time}: {rms}");
        } else {
            assert!(rms > 0.05, "{time}: {rms}");
        }
    }
    let proxy = voxmoviemaker::media::playback::ProxyRequest {
        plan,
        storage: f.storage.clone(),
        ffmpeg: f.settings.ffmpeg.clone(),
        width: 320,
        limit_mb: 8,
        progress: Arc::new(std::sync::atomic::AtomicU32::new(0)),
    };
    assert!(
        voxmoviemaker::media::playback::prepare(&proxy, &|| false, || {})
            .unwrap()
            .path
            .is_file()
    );
}
