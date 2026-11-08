//! Opt-in read-only tests against locally supplied artwork. Never bundle the PSD
//! or derived images: VMM_TEST_PSD must refer to a file outside the repository.
use std::{collections::BTreeSet, path::PathBuf};
use voxmoviemaker::{
    project::FrameRate,
    sprites::{Animation, AnimationFrame, SpeechSpan, Sprite, SpriteRender, psd},
};

#[test]
#[ignore = "requires VMM_TEST_PSD pointing to the user's PSDTool-compatible standing artwork"]
fn actual_psd_mouth_and_blink_replace_variants_without_stacking_them() {
    let path = PathBuf::from(std::env::var_os("VMM_TEST_PSD").expect("set VMM_TEST_PSD"));
    let source = std::fs::read(&path).unwrap();
    assert!(
        !path.canonicalize().unwrap().starts_with(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .canonicalize()
                .unwrap()
        )
    );
    let doc = psd::load(&path).unwrap();
    let nodes = doc.all_nodes();
    let key = |name: &str| {
        nodes
            .iter()
            .find(|n| n.name == name)
            .unwrap_or_else(|| panic!("fixture layer missing: {name}"))
            .key
            .clone()
    };
    let mouth_open = key("*ほあー");
    let mouth_closed = key("*むふ");
    let mouth_other = key("*お");
    let eye_group = key("*目セット");
    let eyes_open = key("*普通目");
    let eyes_white = key("*普通白目");
    let eyes_closed = key("*UU");
    let frame = |layers: Vec<String>| AnimationFrame {
        layers,
        duration_ms: 100,
    };
    let mut settings = Sprite::new("External test".into());
    settings.psd = path.clone();
    settings.blink_interval_ms = 300;
    settings.mouth = Animation {
        enabled: true,
        idle_layers: vec![mouth_closed.clone()],
        frames: vec![
            frame(vec![mouth_closed.clone()]),
            frame(vec![mouth_other.clone()]),
            frame(vec![mouth_closed.clone()]),
        ],
        ..Default::default()
    };
    settings.blink = Animation {
        enabled: true,
        idle_layers: vec![eye_group.clone()],
        frames: vec![
            frame(vec![eye_group.clone()]),
            frame(vec![eyes_closed.clone()]),
            frame(vec![eye_group.clone()]),
        ],
        ..Default::default()
    };
    doc.validate_settings(&settings).unwrap();
    // Configuring frames before choosing an idle pose must not erase the face,
    // even when animations are disabled (the actual reported editing sequence).
    let mut unfinished = SpriteRender {
        voices: Vec::new(),
        settings: Sprite::new("Unfinished".into()),
        source_revision: None,
        speech: Vec::new(),
    };
    let base = doc
        .visibility(&unfinished, 0, 0, FrameRate::default())
        .unwrap();
    unfinished.settings = settings.clone();
    unfinished.settings.mouth.idle_layers.clear();
    unfinished.settings.blink.idle_layers.clear();
    unfinished.settings.mouth.enabled = false;
    unfinished.settings.blink.enabled = false;
    assert_eq!(
        doc.visibility(&unfinished, 0, 0, FrameRate::default())
            .unwrap(),
        base
    );
    unfinished.settings.mouth.enabled = true;
    unfinished.settings.blink.enabled = true;
    assert_eq!(
        doc.visibility(&unfinished, 0, 0, FrameRate::default())
            .unwrap(),
        base
    );
    unfinished.settings.mouth.frames[1].layers.clear();
    unfinished.settings.blink.frames[1].layers.clear();
    unfinished.speech.push(SpeechSpan {
        start: 0,
        end: 1000,
    });
    assert_eq!(
        doc.visibility(
            &unfinished,
            450,
            150,
            FrameRate {
                numerator: 1000,
                denominator: 1
            }
        )
        .unwrap(),
        base
    );
    let render = SpriteRender {
        voices: Vec::new(),
        settings: settings.clone(),
        source_revision: None,
        speech: vec![SpeechSpan {
            start: 100,
            end: 1000,
        }],
    };
    let fps = FrameRate {
        numerator: 1000,
        denominator: 1,
    };
    let initial = doc.visibility(&render, 0, 0, fps).unwrap();
    assert!(initial.contains(&mouth_closed));
    assert!(
        !initial.contains(&mouth_open),
        "initial mouth must be replaced"
    );
    assert!(initial.contains(&eyes_open));
    assert!(initial.contains(&eyes_white));
    let pupil_group = nodes.iter().find(|n| n.name == "!黒目").unwrap();
    let active_pupils = pupil_group
        .children
        .iter()
        .filter(|n| initial.contains(&n.key))
        .count();
    assert_eq!(
        active_pupils, 1,
        "a selected eye group must not enable all its hidden variants"
    );
    let talking = doc.visibility(&render, 150, 150, fps).unwrap();
    assert!(talking.contains(&mouth_other));
    assert!(!talking.contains(&mouth_open));
    assert!(!talking.contains(&mouth_closed));
    let closed = doc.visibility(&render, 450, 450, fps).unwrap();
    assert!(closed.contains(&eyes_closed));
    assert!(!closed.contains(&eye_group));
    assert!(!closed.contains(&eyes_white));
    // An explicitly selected pupil also needs its default white-eye component.
    settings.blink.idle_layers = vec![eyes_open.clone()];
    settings.blink.frames[0].layers = vec![eyes_open.clone()];
    settings.blink.frames[2].layers = vec![eyes_open.clone()];
    let render = SpriteRender {
        voices: Vec::new(),
        settings,
        source_revision: None,
        speech: Vec::new(),
    };
    let open = doc.visibility(&render, 0, 0, fps).unwrap();
    assert!(open.contains(&eyes_white));
    assert!(open.contains(&eyes_open));
    let full = doc.composite(&open).unwrap();
    assert!(full.pixels().any(|p| p[3] > 0));
    let mouth_group = nodes.iter().find(|n| n.name == "!口").unwrap();
    let eye_root = nodes.iter().find(|n| n.name == "!目").unwrap();
    fn parts<'a>(node: &'a psd::Node, keys: &mut BTreeSet<&'a str>) {
        keys.insert(&node.key);
        for child in &node.children {
            parts(child, keys);
        }
    }
    let mut animated = BTreeSet::new();
    parts(mouth_group, &mut animated);
    parts(eye_root, &mut animated);
    for node in nodes.iter().filter(|n| !animated.contains(n.key.as_str())) {
        assert_eq!(
            initial.contains(&node.key),
            talking.contains(&node.key),
            "static part changed: {}",
            node.name
        );
        assert_eq!(
            initial.contains(&node.key),
            closed.contains(&node.key),
            "static part changed: {}",
            node.name
        );
    }
    if let Some(output) = std::env::var_os("VMM_TEST_PSD_OUTPUT").map(PathBuf::from) {
        assert!(output.is_absolute());
        assert!(!output.starts_with(env!("CARGO_MANIFEST_DIR")));
        voxmoviemaker::assets::private_directory(&output).unwrap();
        for (name, visible) in [
            ("idle", &initial),
            ("talking", &talking),
            ("blink", &closed),
        ] {
            image::DynamicImage::ImageRgba8(doc.composite(visible).unwrap())
                .thumbnail(480, 720)
                .save(output.join(format!("{name}.png")))
                .unwrap();
        }
    }
    for n in &nodes {
        let image = doc.element_thumbnail(&n.key, &|| false).unwrap();
        assert!(image.width() <= 64 && image.height() <= 64);
    }
    let before: BTreeSet<_> = doc
        .all_nodes()
        .iter()
        .filter(|n| n.visible)
        .map(|n| n.key.clone())
        .collect();
    assert_eq!(
        before,
        doc.all_nodes()
            .iter()
            .filter(|n| n.visible)
            .map(|n| n.key.clone())
            .collect()
    );
    assert_eq!(
        source,
        std::fs::read(path).unwrap(),
        "source PSD must remain untouched"
    );
}

#[test]
#[ignore = "requires VMM_TEST_PSD and user-installed FFmpeg"]
fn actual_psd_animation_preview_and_export_agree() {
    use std::{
        fs,
        process::Command,
        sync::{Arc, atomic::AtomicU32},
        time::Duration,
    };
    use voxmoviemaker::{
        assets::Storage,
        characters::{Character, CharacterBinding, CharacterLibrary},
        media::{
            cache, export, ffmpeg,
            playback::{self, ProxyRequest},
            render_plan::RenderPlan,
        },
        project::*,
        settings::{ExportFormat, Settings},
    };
    let path = PathBuf::from(std::env::var_os("VMM_TEST_PSD").expect("set VMM_TEST_PSD"));
    assert!(
        !path
            .canonicalize()
            .unwrap()
            .starts_with(env!("CARGO_MANIFEST_DIR"))
    );
    let source = fs::read(&path).unwrap();
    let doc = psd::load(&path).unwrap();
    let key = |name: &str| {
        doc.all_nodes()
            .into_iter()
            .find(|n| n.name == name)
            .unwrap()
            .key
            .clone()
    };
    let root = std::env::temp_dir().join(format!("vmm-external-animation-{}", Id::new_v4()));
    fs::create_dir(&root).unwrap();
    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(root.clone());
    let storage = Storage {
        data: root.join("data"),
        cache: root.join("cache"),
        config: root.join("config"),
    };
    let mut project = Project {
        width: 320,
        height: 488,
        ..Default::default()
    };
    let character = Character::new("Test".into(), 320, 488);
    let frame = |name: &str| AnimationFrame {
        layers: vec![key(name)],
        duration_ms: 100,
    };
    let mut sprite = Sprite::new("External".into());
    sprite.psd = path.clone();
    sprite.character_id = Some(character.id);
    sprite.blink_interval_ms = 300;
    sprite.mouth = Animation {
        enabled: true,
        idle_layers: vec![key("*むふ")],
        frames: vec![frame("*むふ"), frame("*お"), frame("*むふ")],
        ..Default::default()
    };
    sprite.blink = Animation {
        enabled: true,
        idle_layers: vec![key("*目セット")],
        frames: vec![frame("*目セット"), frame("*UU"), frame("*目セット")],
        ..Default::default()
    };
    project.tracks[0].clips.push(Clip {
        id: Id::new_v4(),
        character: None,
        source: ClipSource::Sprite {
            settings_id: sprite.id,
        },
        timeline_start: 0,
        duration: 36,
        source_offset: 0,
        volume: 1.0,
        muted: false,
        effects: Default::default(),
    });
    // Synthetic tone/silence WAV; user artwork remains read-only and external.
    let mut writer = hound::WavWriter::create(
        root.join("timing-only.wav"),
        hound::WavSpec {
            channels: 1,
            sample_rate: 48000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        },
    )
    .unwrap();
    for i in 0..48000 {
        writer
            .write_sample::<i16>(if (9600..19200).contains(&i) {
                0
            } else if i % 2 == 0 {
                8000
            } else {
                -8000
            })
            .unwrap();
    }
    writer.finalize().unwrap();
    let asset = MediaAsset {
        id: Id::new_v4(),
        name: "timing".into(),
        location: AssetLocation::External(root.join("timing-only.wav")),
        kind: MediaKind::Audio,
        duration_seconds: 1.0,
        width: 0,
        height: 0,
        has_audio: true,
        sample_rate: None,
        frame_rate: None,
        codec: String::new(),
        start_time: 0.0,
        rotation: 0,
        video_stream: None,
        audio_stream: None,
    };
    let mut binding = CharacterBinding::new(character.id);
    binding.subtitle_enabled = false;
    project.tracks[1].clips.push(Clip {
        id: Id::new_v4(),
        character: Some(Box::new(binding)),
        source: ClipSource::Voicevox {
            asset_id: asset.id,
            text: String::new(),
            settings: Default::default(),
            subtitle: None,
        },
        timeline_start: 6,
        duration: 18,
        source_offset: 0,
        volume: 1.0,
        muted: false,
        effects: Default::default(),
    });
    project.assets.push(asset);
    let library = voxmoviemaker::sprites::SpriteLibrary {
        schema_version: 1,
        sprites: vec![sprite],
    };
    let characters = CharacterLibrary {
        schema_version: 1,
        characters: vec![character],
    };
    let plan = RenderPlan::with_libraries(&project, &storage, &characters, &library).unwrap();
    let settings = Settings::default();
    let caps = ffmpeg::detect(&settings).unwrap();
    let target = root.join("animation.mp4");
    export::export(
        &plan,
        &settings.ffmpeg,
        &caps,
        ExportFormat::Mp4,
        &target,
        ffmpeg::cancellation(),
        |_| {},
    )
    .unwrap();
    let proxy = playback::prepare(
        &ProxyRequest {
            plan: plan.clone(),
            storage: storage.clone(),
            ffmpeg: settings.ffmpeg.clone(),
            width: 320,
            limit_mb: 32,
            progress: Arc::new(AtomicU32::new(0)),
        },
        &|| false,
        || {},
    )
    .unwrap();
    for frame in 0..36 {
        let preview =
            cache::preview(&plan, &storage, &settings.ffmpeg, frame, 320, 32, || false).unwrap();
        for output in [&target, &proxy.path] {
            let decoded = ffmpeg::run(
                Command::new(&settings.ffmpeg)
                    .args(["-v", "error", "-i"])
                    .arg(output)
                    .args([
                        "-vf",
                        &format!("select=eq(n\\,{frame})"),
                        "-frames:v",
                        "1",
                        "-c:v",
                        "png",
                        "-f",
                        "image2pipe",
                        "pipe:1",
                    ]),
                Duration::from_secs(15),
                || false,
            )
            .unwrap();
            let actual = image::load_from_memory(&decoded.stdout).unwrap().to_rgba8();
            assert_eq!(preview.dimensions(), actual.dimensions());
            for rect in [
                [120, 135, 170, 155],
                [125, 105, 190, 135],
                [80, 180, 240, 380],
            ] {
                let mut difference = 0u64;
                let mut count = 0;
                for y in rect[1]..rect[3] {
                    for x in rect[0]..rect[2] {
                        let a = preview.get_pixel(x, y);
                        let b = actual.get_pixel(x, y);
                        difference += (0..3)
                            .map(|i| (a[i] as i32 - b[i] as i32).unsigned_abs() as u64)
                            .sum::<u64>();
                        count += 3;
                    }
                }
                assert!(
                    (difference as f64 / count as f64) < 9.0,
                    "frame {frame} region {rect:?} diverged in {output:?}: {}",
                    difference as f64 / count as f64
                );
            }
        }
    }
    assert_eq!(source, fs::read(path).unwrap());
}
