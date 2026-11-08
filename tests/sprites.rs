//! Synthetic, independently generated PSDs: no character artwork is distributed.
use apeireth_image_psd::{BlendMode, GroupSpec, LayerSpec, PixelData, PsdBuilder, TaggedBlock};
use std::{collections::BTreeMap, fs, path::PathBuf, sync::Arc};
use voxmoviemaker::{
    assets::Storage,
    characters::{Character, CharacterBinding, CharacterLibrary},
    media::render_plan::RenderPlan,
    project::*,
    sprites::{
        psd::{self, Document},
        *,
    },
};

struct Fixture {
    root: PathBuf,
    storage: Storage,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("vmm-sprites-{}", Id::new_v4()));
        fs::create_dir(&root).unwrap();
        let storage = Storage {
            data: root.join("data"),
            cache: root.join("cache"),
            config: root.join("config"),
        };
        Self { root, storage }
    }
    fn sprite(&self) -> Sprite {
        let path = self.root.join("立ち絵 ' with spaces.psd");
        fs::write(&path, fixture_psd()).unwrap();
        let mut s = Sprite::new("Sprite".into());
        s.psd = path;
        s.mouth.idle_layers = vec!["id:3".into()];
        s.mouth.frames = vec![step("id:3", 100), step("id:4", 100), step("id:3", 100)];
        s.mouth.enabled = true;
        s.blink.idle_layers = vec!["id:5".into()];
        s.blink.frames = vec![step("id:5", 100), step("id:6", 100), step("id:5", 100)];
        s.blink.enabled = true;
        s.blink_interval_ms = 300;
        s
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
fn layer(id: u32, name: &str, x: i32, y: i32, w: u32, h: u32, color: [u8; 4]) -> LayerSpec {
    let mut l = LayerSpec::new(
        name,
        x,
        y,
        w,
        h,
        PixelData::Rgba8(color.repeat((w * h) as usize)),
    );
    l.extra_blocks.push(TaggedBlock::layer_id(id));
    l
}
fn fixture_psd() -> Vec<u8> {
    let mut b = PsdBuilder::new(32, 32);
    b.push_layer(layer(1, "Body", 0, 0, 32, 32, [255, 0, 0, 128]));
    b.begin_group(GroupSpec::new("Face"));
    b.begin_group(GroupSpec::new("Mouth"));
    b.push_layer(layer(3, "閉", 4, 20, 8, 8, [0, 0, 255, 255]));
    let mut open = layer(4, "開", 4, 20, 8, 8, [0, 255, 0, 255]);
    open.visible = false;
    b.push_layer(open);
    b.end_group().unwrap();
    b.begin_group(GroupSpec::new("Eyes"));
    b.push_layer(layer(5, "Open", 20, 4, 8, 8, [255, 255, 255, 255]));
    let mut closed = layer(6, "Closed", 20, 4, 8, 8, [255, 255, 0, 255]);
    closed.visible = false;
    b.push_layer(closed);
    b.end_group().unwrap();
    b.end_group().unwrap();
    let mut file = b.build().unwrap();
    // The builder prepends generated lyid blocks. Replace them in this synthetic
    // fixture so stable test IDs do not collide with its group record IDs.
    for (index, record) in file.layers_mut().iter_mut().enumerate() {
        let id = record
            .blocks
            .iter()
            .rfind(|b| b.key == *b"lyid")
            .and_then(|b| match b.parsed()? {
                Ok(apeireth_image_psd::BlockData::LayerId(id)) => Some(id),
                _ => None,
            })
            .unwrap_or(1000 + index as u32);
        let id = if record.section_type() != apeireth_image_psd::SectionType::Other {
            1000 + index as u32
        } else {
            id
        };
        record.blocks.retain(|b| b.key != *b"lyid");
        record.blocks.push(TaggedBlock::layer_id(id));
    }
    file.to_bytes().unwrap()
}
fn step(key: &str, ms: u32) -> AnimationFrame {
    AnimationFrame {
        layers: vec![key.into()],
        duration_ms: ms,
    }
}
fn clip(source: ClipSource, start: u64, duration: u64) -> Clip {
    Clip {
        id: Id::new_v4(),
        character: None,
        source,
        timeline_start: start,
        duration,
        source_offset: 0,
        volume: 1.0,
        muted: false,
        effects: Default::default(),
    }
}
fn scene(f: &Fixture) -> (Project, SpriteLibrary, CharacterLibrary) {
    let mut p = Project {
        width: 64,
        height: 64,
        ..Default::default()
    };
    let character = Character::new("A".into(), 64, 64);
    let mut s = f.sprite();
    s.character_id = Some(character.id);
    p.tracks[1]
        .clips
        .push(clip(ClipSource::Sprite { settings_id: s.id }, 0, 60));
    let a = MediaAsset {
        id: Id::new_v4(),
        name: "voice".into(),
        location: AssetLocation::Voice(Id::new_v4()),
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
    let mut voice = clip(
        ClipSource::Voicevox {
            asset_id: a.id,
            text: "test".into(),
            settings: Default::default(),
            subtitle: None,
        },
        10,
        20,
    );
    let mut binding = CharacterBinding::new(character.id);
    binding.subtitle_enabled = false;
    voice.character = Some(Box::new(binding));
    p.tracks[0].clips.push(voice);
    let wav = f.storage.resolve(p.project_uuid, &a);
    fs::create_dir_all(wav.parent().unwrap()).unwrap();
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
    for i in 0..96000 {
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
    p.assets.push(a);
    (
        p,
        SpriteLibrary {
            schema_version: 1,
            sprites: vec![s],
        },
        CharacterLibrary {
            schema_version: 1,
            characters: vec![character],
        },
    )
}
#[test]
fn psd_layers_nested_groups_coordinates_alpha_and_cache() {
    let f = Fixture::new();
    let sprite = f.sprite();
    let doc = psd::load(&sprite.psd).unwrap();
    assert_eq!([doc.width, doc.height], [32, 32]);
    assert!(
        doc.nodes
            .iter()
            .any(|n| n.name == "Face" && n.children.len() == 2)
    );
    let n = doc
        .all_nodes()
        .into_iter()
        .find(|n| n.key == "id:3")
        .unwrap();
    assert_eq!(n.name, "閉");
    assert_eq!(n.rect, [4, 20, 12, 28]);
    let pixels = doc.layer_pixels(n.index).unwrap();
    assert_eq!(pixels.get_pixel(0, 0).0, [0, 0, 255, 255]);
    assert!(Arc::ptr_eq(&pixels, &doc.layer_pixels(n.index).unwrap()));
    assert!(Arc::ptr_eq(&doc, &psd::load(&sprite.psd).unwrap()));
    let r = SpriteRender {
        voices: Vec::new(),
        source_revision: None,
        settings: sprite,
        speech: Vec::new(),
    };
    let image = doc
        .composite(&doc.visibility(&r, 0, 0, FrameRate::default()).unwrap())
        .unwrap();
    assert_eq!(image.get_pixel(0, 0).0, [255, 0, 0, 128]);
    assert_eq!(image.get_pixel(8, 24).0, [0, 0, 255, 255]);
    assert_eq!(
        fs::read(&r.settings.psd).unwrap(),
        fixture_psd(),
        "source PSD is read-only"
    );
}

#[test]
fn element_thumbnails_isolate_hidden_variants_groups_and_transparency() {
    let doc = Document::from_bytes(&fixture_psd()).unwrap();
    // A hidden open mouth is still a selectable element; body/eye pixels stay out.
    let mouth = doc.element_thumbnail("id:4", &|| false).unwrap();
    assert!(mouth.width() <= 64 && mouth.height() <= 64);
    assert!(mouth.pixels().all(|p| p.0 == [0, 255, 0, 255]));
    let body = doc.element_thumbnail("id:1", &|| false).unwrap();
    assert!(body.pixels().all(|p| p.0 == [255, 0, 0, 128]));
    let face = doc
        .all_nodes()
        .into_iter()
        .find(|n| n.name == "Face")
        .unwrap();
    let group = doc.element_thumbnail(&face.key, &|| false).unwrap();
    assert!(group.pixels().any(|p| p[3] == 0));
    assert!(group.pixels().any(|p| p[2] > 200 && p[0] < 10));
    assert!(
        group
            .pixels()
            .any(|p| p[0] > 200 && p[1] > 200 && p[2] > 200)
    );
    assert!(
        !group
            .pixels()
            .any(|p| p[0] > 200 && p[1] < 10 && p[2] < 10 && p[3] > 0)
    );
    assert!(doc.element_thumbnail("missing", &|| false).is_err());
    assert_eq!(
        doc.element_thumbnail("id:4", &|| true).unwrap_err(),
        "cancelled"
    );
}

#[test]
fn animation_groups_keep_initial_children_and_replace_psdtool_alternatives() {
    let mut b = PsdBuilder::new(32, 32);
    b.push_layer(layer(1, "Body", 0, 0, 32, 32, [255, 0, 0, 255]));
    b.begin_group(GroupSpec::new("!Mouth"));
    b.push_layer(layer(2, "*Open mouth", 2, 20, 6, 4, [0, 255, 0, 255]));
    let mut closed = layer(3, "*Closed mouth", 2, 20, 6, 4, [0, 0, 255, 255]);
    closed.visible = false;
    b.push_layer(closed);
    b.end_group().unwrap();
    b.begin_group(GroupSpec::new("!Eyes"));
    let mut closed = layer(4, "*Closed eyes", 16, 2, 8, 8, [0, 0, 0, 255]);
    closed.visible = false;
    b.push_layer(closed);
    b.begin_group(GroupSpec::new("*Open eye set"));
    b.push_layer(layer(5, "White", 16, 2, 8, 8, [255, 255, 255, 255]));
    b.begin_group(GroupSpec::new("!Pupils"));
    b.push_layer(layer(6, "*Pupil A", 18, 4, 2, 2, [0, 0, 0, 255]));
    let mut other = layer(7, "*Pupil B", 20, 4, 2, 2, [0, 255, 255, 255]);
    other.visible = false;
    b.push_layer(other);
    b.end_group().unwrap();
    b.end_group().unwrap();
    b.end_group().unwrap();
    let doc = Document::from_bytes(&b.to_bytes().unwrap()).unwrap();
    let nodes = doc.all_nodes();
    let key = |name: &str| nodes.iter().find(|n| n.name == name).unwrap().key.clone();
    let mut s = Sprite::new("test".into());
    s.mouth = Animation {
        enabled: true,
        idle_layers: vec![key("*Closed mouth")],
        frames: vec![step(&key("*Closed mouth"), 100)],
        ..Default::default()
    };
    s.blink = Animation {
        enabled: true,
        idle_layers: vec![key("*Open eye set")],
        frames: vec![
            step(&key("*Open eye set"), 100),
            step(&key("*Closed eyes"), 100),
            step(&key("*Open eye set"), 100),
        ],
        ..Default::default()
    };
    s.blink_interval_ms = 100;
    doc.validate_settings(&s).unwrap();
    let mut render = SpriteRender {
        voices: Vec::new(),
        source_revision: None,
        settings: s,
        speech: Vec::new(),
    };
    let fps = FrameRate {
        numerator: 1000,
        denominator: 1,
    };
    let open = doc.visibility(&render, 0, 0, fps).unwrap();
    assert!(!open.contains(&key("*Open mouth")));
    assert!(open.contains(&key("*Pupil A")));
    assert!(!open.contains(&key("*Pupil B")));
    let image = doc.composite(&open).unwrap();
    assert_eq!(image.get_pixel(3, 21).0, [0, 0, 255, 255]);
    assert_eq!(image.get_pixel(0, 31).0, [255, 0, 0, 255]);
    let closed = doc.visibility(&render, 250, 250, fps).unwrap();
    assert!(!closed.contains(&key("*Open eye set")));
    assert!(!closed.contains(&key("White")));
    assert!(closed.contains(&key("*Closed eyes")));
    render.settings.blink.idle_layers = vec![key("*Pupil B")];
    let open = doc.visibility(&render, 0, 0, fps).unwrap();
    assert!(open.contains(&key("White")));
    assert!(open.contains(&key("*Pupil B")));
    assert!(!open.contains(&key("*Pupil A")));
    render.settings.mouth.idle_layers = vec![key("*Pupil A")];
    assert_eq!(
        doc.validate_settings(&render.settings).unwrap_err(),
        "sprite-animation-conflict"
    );
}

#[test]
fn unfinished_animation_settings_preserve_base_face_parts() {
    let doc = Document::from_bytes(&fixture_psd()).unwrap();
    let mut render = SpriteRender {
        voices: Vec::new(),
        settings: Sprite::new("test".into()),
        source_revision: None,
        speech: vec![SpeechSpan {
            start: 100,
            end: 400,
        }],
    };
    let fps = FrameRate {
        numerator: 1000,
        denominator: 1,
    };
    let baseline = doc.visibility(&render, 0, 0, fps).unwrap();
    render.settings.mouth.frames = vec![
        step("id:3", 100),
        step("id:4", 100),
        AnimationFrame::default(),
    ];
    render.settings.blink.frames = vec![
        step("id:6", 100),
        step("id:5", 100),
        AnimationFrame::default(),
    ];
    render.settings.blink_interval_ms = 100;
    for frame in [0, 150, 350, 450] {
        assert_eq!(
            doc.visibility(&render, frame, frame, fps).unwrap(),
            baseline
        );
    }
    render.settings.mouth.enabled = true;
    render.settings.blink.enabled = true;
    // Before speech/blinking and after speech.
    for frame in [0, 750] {
        assert_eq!(
            doc.visibility(&render, frame, frame, fps).unwrap(),
            baseline
        );
    }
    let active = doc.visibility(&render, 150, 150, fps).unwrap();
    assert!(active.contains("id:4") && active.contains("id:6"));
    render.settings.mouth.frames[1].layers.clear();
    // Explicit idle choices are also used by unfinished poses.
    render.settings.mouth.idle_layers = vec!["id:4".into()];
    render.settings.blink.idle_layers = vec!["id:6".into()];
    let idle = doc.visibility(&render, 350, 350, fps).unwrap();
    assert!(idle.contains("id:4") && idle.contains("id:6"));
    // Base visibility overrides remain authoritative when idle is unspecified.
    render.settings.mouth.idle_layers.clear();
    render.settings.blink.idle_layers.clear();
    render.settings.visibility.insert("id:3".into(), false);
    render.settings.visibility.insert("id:4".into(), true);
    let idle = doc.visibility(&render, 350, 350, fps).unwrap();
    assert!(!idle.contains("id:3") && idle.contains("id:4"));
    assert!(idle.contains("id:5"));
}

#[test]
fn element_thumbnails_preserve_tall_and_wide_aspect_ratios() {
    let mut b = PsdBuilder::new(32, 32);
    b.push_layer(layer(1, "Tall", 0, 0, 2, 20, [0, 255, 0, 255]));
    b.push_layer(layer(2, "Wide", 0, 22, 20, 2, [0, 0, 255, 255]));
    let doc = Document::from_bytes(&b.to_bytes().unwrap()).unwrap();
    for (name, expected) in [("Tall", (2, 20)), ("Wide", (20, 2))] {
        let key = doc
            .all_nodes()
            .into_iter()
            .find(|n| n.name == name)
            .unwrap()
            .key
            .clone();
        assert_eq!(
            doc.element_thumbnail(&key, &|| false).unwrap().dimensions(),
            expected
        );
    }
}
#[test]
fn group_opacity_is_applied_after_children_are_composited() {
    let mut b = PsdBuilder::new(2, 2);
    let mut group = GroupSpec::new("normal");
    group.blend_mode = BlendMode::Normal;
    group.opacity = 128;
    b.begin_group(group);
    b.push_layer(layer(1, "red", 0, 0, 2, 2, [255, 0, 0, 255]));
    b.push_layer(layer(2, "blue", 0, 0, 2, 2, [0, 0, 255, 255]));
    b.end_group().unwrap();
    let doc = Document::from_bytes(&b.to_bytes().unwrap()).unwrap();
    let visible = doc.all_nodes().iter().map(|n| n.key.clone()).collect();
    assert_eq!(
        doc.composite(&visible).unwrap().get_pixel(0, 0).0,
        [0, 0, 255, 128]
    );
}
#[test]
fn arbitrary_animation_lengths_speed_repeat_and_idle_are_deterministic() {
    let fps = FrameRate {
        numerator: 1000,
        denominator: 1,
    };
    for count in [3, 4, 5, 8] {
        let mut a = Animation {
            enabled: true,
            frames: (0..count).map(|i| step(&i.to_string(), 100)).collect(),
            ..Default::default()
        };
        a.idle_layers = vec!["idle".into()];
        for i in 0..count {
            assert_eq!(a.layers_at(i * 100 + 50, fps, true, None), [i.to_string()]);
        }
        assert_eq!(a.layers_at(count * 100, fps, true, None), ["0"]);
        assert_eq!(a.layers_at(150, fps, false, None), ["idle"]);
        a.speed_milli = 2000;
        assert_eq!(a.layers_at(75, fps, true, None), ["1"]);
        a.repeat = false;
        assert_eq!(
            a.layers_at(100000, fps, true, None),
            [(count - 1).to_string()]
        );
        a.enabled = false;
        assert_eq!(a.layers_at(150, fps, true, None), ["idle"]);
    }
}
#[test]
fn blink_three_and_five_frames_runs_without_speech_and_returns_to_idle() {
    let fps = FrameRate {
        numerator: 1000,
        denominator: 1,
    };
    for count in [3, 5] {
        let a = Animation {
            enabled: true,
            idle_layers: vec!["open".into()],
            frames: (0..count).map(|i| step(&i.to_string(), 100)).collect(),
            ..Default::default()
        };
        assert_eq!(a.layers_at(299, fps, true, Some(300)), ["open"]);
        for i in 0..count {
            assert_eq!(
                a.layers_at(300 + i * 100, fps, true, Some(300)),
                [i.to_string()]
            );
        }
        assert_eq!(
            a.layers_at(300 + count * 100, fps, true, Some(300)),
            ["open"]
        );
    }
}
#[test]
fn voice_overlap_character_identity_union_and_idle_mouth_are_independent_of_eyes() {
    let f = Fixture::new();
    let (mut p, library, _) = scene(&f);
    let s = &library.sprites[0];
    let original = p.tracks[0].clips[0].clone();
    let mut second = original.clone();
    second.id = Id::new_v4();
    second.timeline_start = 20;
    second.duration = 30;
    p.tracks[2].clips.push(second);
    let mut other = original;
    other.id = Id::new_v4();
    other.timeline_start = 1;
    other.character.as_mut().unwrap().character_id = Id::new_v4();
    p.tracks[3].clips.push(other);
    let spans = speech_spans(&p, s.character_id);
    assert_eq!(spans.len(), 1);
    assert_eq!((spans[0].start, spans[0].end), (10, 50));
    let r = SpriteRender {
        voices: Vec::new(),
        source_revision: None,
        settings: s.clone(),
        speech: spans,
    };
    let fps = p.frame_rate;
    assert!(r.selected_layers(9, 9, fps).0.contains("id:3"));
    assert!(r.selected_layers(14, 14, fps).0.contains("id:4"));
    assert!(r.selected_layers(50, 50, fps).0.contains("id:3"));
    assert!(
        r.selected_layers(13, 13, fps).1.contains("id:6"),
        "blink runs with its own clip time"
    );
    p.tracks[0].muted = true;
    p.tracks[2].muted = true;
    assert!(speech_spans(&p, s.character_id).is_empty());
}
#[test]
fn rational_animation_boundaries_match_seeks_and_do_not_accumulate_error() {
    for fps in [
        FrameRate {
            numerator: 24000,
            denominator: 1001,
        },
        FrameRate {
            numerator: 30000,
            denominator: 1001,
        },
    ] {
        let animation = Animation {
            enabled: true,
            frames: vec![
                step("a", 73),
                step("b", 129),
                step("c", 57),
                step("d", 81),
                step("e", 100),
            ],
            ..Default::default()
        };
        let mut frame = 0;
        while frame < 120000 {
            let next = animation.next_change(frame, fps, None);
            assert!(next > frame);
            for inside in frame..next.min(120000) {
                assert_eq!(
                    animation.layers_at(frame, fps, true, None),
                    animation.layers_at(inside, fps, true, None)
                );
            }
            frame = next;
        }
    }
}
#[test]
fn unrelated_body_visibility_is_unchanged_and_overlapping_animation_layers_are_rejected() {
    let f = Fixture::new();
    let mut s = f.sprite();
    let doc = psd::load(&s.psd).unwrap();
    doc.validate_settings(&s).unwrap();
    let r = SpriteRender {
        voices: Vec::new(),
        source_revision: None,
        settings: s.clone(),
        speech: vec![SpeechSpan { start: 0, end: 100 }],
    };
    for frame in [0, 4, 7, 13, 22, 99] {
        assert!(
            doc.visibility(&r, frame, frame, FrameRate::default())
                .unwrap()
                .contains("id:1")
        );
    }
    s.blink.frames[0].layers.push("id:3".into());
    assert_eq!(
        doc.validate_settings(&s).unwrap_err(),
        "sprite-animation-conflict"
    );
    s.blink.frames[0].layers = vec!["missing".into()];
    assert!(
        doc.validate_settings(&s)
            .unwrap_err()
            .starts_with("sprite-layer-missing")
    );
}
#[test]
fn shared_xdg_settings_round_trip_conflicts_corruption_and_multiple_projects() {
    let f = Fixture::new();
    let s = f.sprite();
    let id = s.id;
    let mut store = SpriteStore::load(&f.storage.config);
    let mut stale = SpriteStore::load(&f.storage.config);
    store
        .save(SpriteLibrary {
            schema_version: 1,
            sprites: vec![s],
        })
        .unwrap();
    assert_eq!(SpriteStore::load(&f.storage.config).library, store.library);
    assert_eq!(
        stale.save(SpriteLibrary::default()).unwrap_err(),
        "sprites-conflict"
    );
    let (p, _, characters) = scene(&f);
    let mut p = p;
    p.tracks[1].clips[0].source = ClipSource::Sprite { settings_id: id };
    let p2 = p.clone();
    for project in [&p, &p2] {
        assert_eq!(
            RenderPlan::with_characters(project, &f.storage, &characters)
                .unwrap()
                .clips
                .iter()
                .find_map(|c| c.sprite.as_ref())
                .unwrap()
                .settings
                .id,
            id
        );
    }
    let mut changed = store.library.clone();
    changed.sprites[0].name = "Changed globally".into();
    store.save(changed).unwrap();
    assert_eq!(
        RenderPlan::with_characters(&p2, &f.storage, &characters)
            .unwrap()
            .clips
            .iter()
            .find_map(|c| c.sprite.as_ref())
            .unwrap()
            .settings
            .name,
        "Changed globally"
    );
    fs::write(f.storage.config.join("sprites.json"), b"broken json").unwrap();
    let mut corrupt = SpriteStore::load(&f.storage.config);
    assert!(corrupt.error.is_some());
    assert!(corrupt.save(SpriteLibrary::default()).is_err());
    assert_eq!(
        fs::read(f.storage.config.join("sprites.json")).unwrap(),
        b"broken json"
    );
}
#[test]
fn project_contains_only_sprite_id_and_placement_and_keeps_legacy_compatibility() {
    let f = Fixture::new();
    let (mut p, library, characters) = scene(&f);
    let json = serde_json::to_string(&p).unwrap();
    assert!(!json.contains("blink_interval"));
    assert!(!json.contains(".psd"));
    assert!(!json.contains("idle_layers"));
    let path = f.root.join("old.vmm.json");
    p.schema_version = 4;
    fs::write(&path, serde_json::to_vec(&p).unwrap()).unwrap();
    let restored = persistence::load(&path).unwrap();
    assert_eq!(restored.schema_version, SCHEMA_VERSION);
    assert!(RenderPlan::with_libraries(&restored, &f.storage, &characters, &library).is_ok());
    assert!(
        RenderPlan::with_libraries(
            &restored,
            &f.storage,
            &characters,
            &SpriteLibrary::default()
        )
        .unwrap_err()
        .starts_with("sprite-missing")
    );
}
#[test]
fn move_split_trim_duplicate_and_reorder_preserve_sprite_reference_and_phase() {
    let f = Fixture::new();
    let (mut p, library, characters) = scene(&f);
    let id = p.tracks[1].clips[0].id;
    commands::split(&mut p, id, 25).unwrap();
    let right = p.tracks[1]
        .clips
        .iter()
        .find(|c| c.id != id)
        .unwrap()
        .clone();
    assert_eq!(right.source_offset, 25);
    assert_eq!(right.source, p.clip(id).unwrap().source);
    commands::move_clip(&mut p, right.id, 2, 40).unwrap();
    p.tracks.swap(1, 2);
    p.validate().unwrap();
    let plan = RenderPlan::with_libraries(&p, &f.storage, &characters, &library).unwrap();
    let rc = plan.clips.iter().find(|c| c.id == right.id).unwrap();
    assert_eq!(rc.offset, 25);
    let r = rc.sprite.as_ref().unwrap();
    assert_eq!(
        r.selected_layers(25, 40, p.frame_rate).1,
        r.selected_layers(rc.offset, rc.start, p.frame_rate).1
    );
    let mut history = commands::History::default();
    let before = p.clone();
    history.edit(&mut p, |p| commands::delete(p, id)).unwrap();
    history.undo(&mut p);
    assert_eq!(p, before);
}
#[test]
fn missing_psd_unsupported_features_invalid_input_and_source_changes_are_safe() {
    let f = Fixture::new();
    let s = f.sprite();
    let old = psd::load(&s.psd).unwrap();
    assert!(Document::from_bytes(b"invalid PSD").is_err());
    let mut b = PsdBuilder::new(32, 32);
    let mut l = layer(1, "Multiply", 0, 0, 1, 1, [0, 0, 0, 255]);
    l.blend_mode = BlendMode::Multiply;
    b.push_layer(l);
    assert!(Document::from_bytes(&b.to_bytes().unwrap()).is_err());
    let mut b = PsdBuilder::new(32, 32);
    b.push_layer(layer(99, "replacement", 0, 0, 1, 1, [1, 2, 3, 255]));
    fs::write(&s.psd, b.to_bytes().unwrap()).unwrap();
    let new = psd::load(&s.psd).unwrap();
    assert!(!Arc::ptr_eq(&old, &new));
    assert!(new.validate_settings(&s).is_err());
    fs::remove_file(&s.psd).unwrap();
    assert!(psd::load(&s.psd).is_err());
}
#[test]
fn sprite_compilation_is_bounded_cancellable_and_cleans_disposable_files() {
    let f = Fixture::new();
    let (mut p, library, characters) = scene(&f);
    p.tracks[0].clips.clear();
    p.assets.clear();
    let plan = RenderPlan::with_libraries(&p, &f.storage, &characters, &library).unwrap();
    assert_eq!(
        plan.compile_cancellable(None, None, &|| true)
            .err()
            .unwrap(),
        "cancelled"
    );
    {
        let compiled = plan.compile(None).unwrap();
        let args = compiled
            .args
            .iter()
            .map(|s| s.to_string_lossy())
            .collect::<Vec<_>>()
            .join(" ");
        assert!(args.contains("concat"));
        assert!(fs::read_dir(&plan.scratch).unwrap().next().is_some());
    }
    assert!(fs::read_dir(&plan.scratch).unwrap().next().is_none());
    let mut invalid = library.clone();
    invalid.sprites[0].visibility = BTreeMap::from([("missing".into(), true)]);
    let invalid = RenderPlan::with_libraries(&p, &f.storage, &characters, &invalid).unwrap();
    assert!(invalid.compile(None).is_err());
    assert!(fs::read_dir(&plan.scratch).unwrap().next().is_none());
}

#[test]
#[ignore = "requires user-installed FFmpeg with libx264, AAC, mpeg4 and PCM"]
fn real_sprite_preview_proxy_mp4_and_ntsc_export_agree_at_animation_boundaries() {
    use std::{process::Command, sync::atomic::AtomicU32, time::Duration};
    use voxmoviemaker::{
        media::{
            cache, export, ffmpeg,
            playback::{self, ProxyRequest},
        },
        settings::{ExportFormat, Settings},
    };
    let f = Fixture::new();
    let (mut p, library, characters) = scene(&f);
    // Real synthetic WAV includes an internal silent interval for mouth gating.
    let settings = Settings::default();
    let capabilities = ffmpeg::detect(&settings).unwrap();
    for (iteration, fps) in [
        FrameRate::default(),
        FrameRate {
            numerator: 30000,
            denominator: 1001,
        },
        FrameRate {
            numerator: 24000,
            denominator: 1001,
        },
    ]
    .into_iter()
    .enumerate()
    {
        p.frame_rate = fps;
        let plan = RenderPlan::with_libraries(&p, &f.storage, &characters, &library).unwrap();
        let target = f.root.join(format!("export-{iteration}.mp4"));
        export::export(
            &plan,
            &settings.ffmpeg,
            &capabilities,
            ExportFormat::Mp4,
            &target,
            ffmpeg::cancellation(),
            |_| {},
        )
        .unwrap();
        let proxy = playback::prepare(
            &ProxyRequest {
                plan: plan.clone(),
                storage: f.storage.clone(),
                ffmpeg: settings.ffmpeg.clone(),
                width: 160,
                limit_mb: 32,
                progress: Arc::new(AtomicU32::new(0)),
            },
            &|| false,
            || {},
        )
        .unwrap();
        let decode = |path: &std::path::Path, frame: u64| {
            let out = ffmpeg::run(
                Command::new(&settings.ffmpeg)
                    .args(["-v", "error", "-i"])
                    .arg(path)
                    .args([
                        "-vf",
                        &format!("select=eq(n\\,{frame}),scale=160:160"),
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
            image::load_from_memory(&out.stdout).unwrap().to_rgba8()
        };
        for frame in 0..60 {
            let preview =
                cache::preview(&plan, &f.storage, &settings.ffmpeg, frame, 160, 16, || {
                    false
                })
                .unwrap();
            for path in [&target, &proxy.path] {
                let encoded = decode(path, frame);
                for (x, y) in [(40, 120), (120, 40), (5, 5)] {
                    let a = preview.get_pixel(x, y);
                    let b = encoded.get_pixel(x, y);
                    assert!(
                        (0..3).all(|i| (a[i] as i32 - b[i] as i32).abs() < 30),
                        "fps={fps:?} frame={frame} {path:?} sample={x},{y}: {a:?} != {b:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn voice_gating_uses_matching_enabled_clips_and_preserves_legacy_settings() {
    let f = Fixture::new();
    let (mut p, library, characters) = scene(&f);
    let sprite = &library.sprites[0];
    let mut json = serde_json::to_value(sprite).unwrap();
    json.as_object_mut().unwrap().remove("mouth_detection");
    let legacy: Sprite = serde_json::from_value(json).unwrap();
    assert_eq!(legacy.mouth_detection, MouthDetection::default());
    assert_eq!(legacy.open_mouth(), ["id:4"]);
    let mut other = p.tracks[0].clips[0].clone();
    other.id = Id::new_v4();
    other.character.as_mut().unwrap().character_id = Id::new_v4();
    p.tracks[2].clips.push(other);
    let build = |p: &Project| {
        RenderPlan::with_libraries(p, &f.storage, &characters, &library)
            .unwrap()
            .clips
            .into_iter()
            .find_map(|c| c.sprite)
            .unwrap()
    };
    assert_eq!(build(&p).voices.len(), 1);
    p.tracks[0].clips[0].source_offset = 3;
    p.tracks[0].clips[0].timeline_start = 15;
    let snapshot = build(&p);
    assert_eq!(
        (snapshot.voices[0].offset, snapshot.voices[0].start),
        (3, 15)
    );
    let resolved = speech::resolve(
        &snapshot,
        p.frame_rate,
        &f.storage.cache.join("speech"),
        &|| false,
    )
    .unwrap();
    assert!(!resolved.speech.is_empty());
    // Every active frame uses exactly the same open pose, regardless of seek order.
    for frame in (0..60).rev() {
        let expected = if resolved
            .speech
            .iter()
            .any(|s| s.start <= frame && frame < s.end)
        {
            "id:4"
        } else {
            "id:3"
        };
        assert!(
            resolved
                .selected_layers(frame, frame, p.frame_rate)
                .0
                .contains(expected)
        );
        let next = frame
            .saturating_add(resolved.next_change(frame, frame, p.frame_rate))
            .min(60);
        let pose = resolved.selected_layers(frame, frame, p.frame_rate);
        for inside in frame..next {
            assert_eq!(resolved.selected_layers(inside, inside, p.frame_rate), pose);
        }
    }
    p.tracks[0].clips[0].muted = true;
    assert!(build(&p).voices.is_empty());
    p.tracks[0].clips[0].muted = false;
    p.tracks[0].muted = true;
    assert!(build(&p).voices.is_empty());
    p.tracks[0].muted = false;
    p.tracks[0].enabled = false;
    assert!(build(&p).voices.is_empty());
    p.tracks[0].enabled = true;
    p.tracks[0].clips[0].volume = 0.0;
    assert!(build(&p).voices.is_empty());
}
