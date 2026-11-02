use std::{fs, path::PathBuf};
use voxmoviemaker::{
    assets::{Storage, xdg_path},
    characters::*,
    media::{render_plan::RenderPlan, text},
    project::{persistence, *},
};
struct Fixture {
    root: PathBuf,
    storage: Storage,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("vmm-characters-{}", Id::new_v4()));
        fs::create_dir(&root).unwrap();
        let storage = Storage {
            data: root.join("data"),
            cache: root.join("cache"),
            config: root.join("config"),
        };
        Self { root, storage }
    }
    fn library(&self) -> CharacterLibrary {
        let mut character = Character::new("Narrator".into(), 1280, 720);
        character.voice.speaker = 3;
        character.voice.speaker_uuid = "speaker-uuid".into();
        character.voice.speaker_name = "VOICEVOX speaker".into();
        character.voice.style_name = "Normal".into();
        character.voice.speed = 1.2;
        CharacterLibrary {
            schema_version: 1,
            characters: vec![character],
        }
    }
    fn project(&self, id: Id) -> Project {
        let mut project = Project::default();
        project.tracks[0].clips.push(Clip {
            id: Id::new_v4(),
            character: Some(Box::new(CharacterBinding::new(id))),
            source: ClipSource::Text {
                settings: TextSettings {
                    text: "字幕".into(),
                    ..Default::default()
                },
            },
            timeline_start: 0,
            duration: 30,
            source_offset: 0,
            volume: 1.0,
            muted: false,
            effects: Default::default(),
        });
        project
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn same_names_and_same_voice_keep_distinct_character_settings() {
    let f = Fixture::new();
    let mut library = f.library();
    let mut copy = library.characters[0].clone();
    copy.id = Id::new_v4();
    copy.voice.speed = 0.9;
    copy.subtitle.size = 36.0;
    library.characters.push(copy);
    library.validate().unwrap();
    assert_ne!(library.characters[0].id, library.characters[1].id);
    for i in 0..2 {
        let binding = CharacterBinding::new(library.characters[i].id);
        assert_eq!(
            binding.voice(&library).unwrap().speed,
            library.characters[i].voice.speed
        );
    }
    library.characters[1].id = library.characters[0].id;
    assert!(library.validate().is_err());
}

#[test]
fn sparse_overrides_inherit_independently_and_preserve_zero_false_empty_and_null() {
    let f = Fixture::new();
    let mut library = f.library();
    let id = library.characters[0].id;
    let mut binding = CharacterBinding::new(id);
    binding.voice.speed = Some(1.5);
    binding.voice.pitch = Some(0.0);
    binding.voice.stereo = Some(false);
    binding.voice.pause_length = Some(Nullable { value: None });
    binding.subtitle.size = Some(72.0);
    binding.subtitle.x = Some(0.0);
    binding.subtitle.font = Some(PathBuf::new());
    library.characters[0].voice.intonation = 1.8;
    library.characters[0].voice.stereo = true;
    library.characters[0].voice.pause_length = Some(0.8);
    library.characters[0].subtitle.position = Some([960.0, 950.0]);
    library.characters[0].subtitle.size = 48.0;
    let voice = binding.voice(&library).unwrap();
    assert_eq!(voice.speed, 1.5);
    assert_eq!(voice.intonation, 1.8);
    assert_eq!(voice.pause_length, None);
    assert!(!voice.stereo);
    let text = binding.text(&library, "phrase");
    assert_eq!(text.position, Some([0.0, 950.0]));
    assert_eq!(text.size, 72.0);
    assert!(text.font.as_os_str().is_empty());
    let json = serde_json::to_value(&binding).unwrap();
    assert_eq!(json["voice"]["pitch"], 0.0);
    assert_eq!(json["voice"]["stereo"], false);
    assert!(json["voice"].get("intonation").is_none());
    let restored: CharacterBinding = serde_json::from_value(json).unwrap();
    assert_eq!(restored, binding);
    binding.voice = Default::default();
    binding.subtitle = Default::default();
    assert_eq!(binding.voice(&library).unwrap().speed, 1.2);
    assert_eq!(binding.text(&library, "phrase").size, 48.0);
}

#[test]
fn character_storage_persists_across_projects_and_rejects_stale_writes() {
    let f = Fixture::new();
    let mut first = CharacterStore::load(&f.storage.config);
    let mut stale = CharacterStore::load(&f.storage.config);
    let library = f.library();
    let id = library.characters[0].id;
    first.save(library.clone()).unwrap();
    assert_eq!(stale.save(f.library()).unwrap_err(), "characters-conflict");
    assert_eq!(CharacterStore::load(&f.storage.config).library, library);
    let a = f.project(id);
    let b = f.project(id);
    let before = serde_json::to_string(&a).unwrap();
    let mut updated = first.library.clone();
    updated.characters[0].subtitle.size = 65.0;
    first.save(updated).unwrap();
    for project in [&a, &b] {
        let plan = RenderPlan::new(project, &f.storage).unwrap();
        assert_eq!(plan.clips[0].text.as_ref().unwrap().size, 65.0);
    }
    assert_eq!(
        serde_json::to_string(&a).unwrap(),
        before,
        "shared changes must not rewrite project data"
    );
}

#[test]
fn corrupt_unknown_schema_and_unreadable_character_files_are_never_overwritten() {
    let f = Fixture::new();
    fs::create_dir_all(&f.storage.config).unwrap();
    let path = f.storage.config.join("characters.json");
    for original in ["{broken", "{\"schema_version\":99,\"characters\":[]}"] {
        fs::write(&path, original).unwrap();
        let mut store = CharacterStore::load(&f.storage.config);
        assert!(store.error.is_some());
        assert_eq!(store.save(f.library()).unwrap_err(), "characters-protected");
        assert_eq!(fs::read_to_string(&path).unwrap(), original);
    }
    fs::remove_file(&path).unwrap();
    fs::create_dir(&path).unwrap();
    let mut store = CharacterStore::load(&f.storage.config);
    assert!(store.error.is_some());
    assert!(store.save(f.library()).is_err());
    assert!(path.is_dir());
}

#[test]
fn concurrent_character_saves_keep_one_complete_library() {
    use std::sync::{Arc, Barrier};
    let f = Fixture::new();
    let first = CharacterStore::load(&f.storage.config);
    let second = CharacterStore::load(&f.storage.config);
    let gate = Arc::new(Barrier::new(2));
    let library = f.library();
    let handles: Vec<_> = [first, second]
        .into_iter()
        .enumerate()
        .map(|(i, mut store)| {
            let gate = gate.clone();
            let mut library = library.clone();
            library.characters[0].name = format!("Writer {i}");
            std::thread::spawn(move || {
                gate.wait();
                store.save(library)
            })
        })
        .collect();
    let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    assert!(
        results
            .iter()
            .filter_map(|r| r.as_ref().err())
            .all(|e| e == "characters-busy" || e == "characters-conflict")
    );
    let restored = CharacterStore::load(&f.storage.config);
    assert!(restored.error.is_none());
    assert_eq!(restored.library.characters.len(), 1);
}

#[test]
fn failed_character_write_preserves_memory_and_can_be_retried() {
    let f = Fixture::new();
    let mut store = CharacterStore::load(&f.storage.config);
    fs::create_dir_all(f.storage.config.join("characters.lock")).unwrap();
    assert!(store.save(f.library()).is_err());
    assert!(store.library.characters.is_empty());
    assert!(!f.storage.config.join("characters.json").exists());
    fs::remove_dir(f.storage.config.join("characters.lock")).unwrap();
    store.save(f.library()).unwrap();
    assert_eq!(store.library.characters.len(), 1);
}

#[test]
fn missing_character_references_survive_reload_and_resolve_again_when_restored() {
    let f = Fixture::new();
    let library = f.library();
    let id = library.characters[0].id;
    let mut p = f.project(id);
    p.tracks[0].clips[0]
        .character
        .as_mut()
        .unwrap()
        .subtitle
        .size = Some(80.0);
    let path = f.root.join("project.vmm.json");
    persistence::save(&path, &p).unwrap();
    let json = fs::read_to_string(&path).unwrap();
    assert!(!json.contains("Narrator"));
    assert!(!json.contains("speaker-uuid"));
    let p = persistence::load(&path).unwrap();
    let binding = p.tracks[0].clips[0].character.as_ref().unwrap();
    assert_eq!(binding.character_id, id);
    assert_eq!(
        binding.voice(&CharacterLibrary::default()).unwrap_err(),
        "character-missing"
    );
    RenderPlan::new(&p, &f.storage).unwrap();
    CharacterStore::load(&f.storage.config)
        .save(library)
        .unwrap();
    let plan = RenderPlan::new(&p, &f.storage).unwrap();
    assert_eq!(plan.clips[0].text.as_ref().unwrap().size, 80.0);
}

#[test]
fn legacy_schema_three_settings_stay_local_and_no_characters_are_created() {
    let f = Fixture::new();
    let mut p = f.project(Id::new_v4());
    p.tracks[0].clips[0].character = None;
    let mut json = serde_json::to_value(&p).unwrap();
    json["schema_version"] = 3.into();
    json["tracks"][0]["clips"][0]
        .as_object_mut()
        .unwrap()
        .remove("character");
    let path = f.root.join("legacy.vmm.json");
    fs::write(&path, serde_json::to_vec(&json).unwrap()).unwrap();
    let restored = persistence::load(&path).unwrap();
    assert_eq!(restored.schema_version, SCHEMA_VERSION);
    assert!(restored.tracks[0].clips[0].character.is_none());
    assert!(!f.storage.config.join("characters.json").exists());
    assert_eq!(
        restored.tracks[0].clips[0].source,
        p.tracks[0].clips[0].source
    );
}

#[test]
fn pixel_subtitle_positions_scale_with_preview_without_changing_settings() {
    let settings = TextSettings {
        text: "Test".into(),
        position: Some([320.0, 180.0]),
        anchor: TextAnchor::Center,
        ..Default::default()
    };
    let before = settings.clone();
    fn center(image: &image::RgbaImage) -> [f64; 2] {
        let mut b = [image.width(), image.height(), 0, 0];
        for (x, y, p) in image.enumerate_pixels().filter(|(_, _, p)| p[3] > 0) {
            let _ = p;
            b[0] = b[0].min(x);
            b[1] = b[1].min(y);
            b[2] = b[2].max(x);
            b[3] = b[3].max(y);
        }
        [(b[0] + b[2]) as f64 / 2.0, (b[1] + b[3]) as f64 / 2.0]
    }
    let full = center(&text::rasterize(&settings, 640, 360, 640).unwrap());
    let half = center(&text::rasterize(&settings, 320, 180, 640).unwrap());
    for i in 0..2 {
        assert!((full[i] - half[i] * 2.0).abs() < 3.0);
    }
    assert!((full[0] - 320.0).abs() < 10.0);
    assert!((full[1] - 180.0).abs() < 15.0);
    assert_eq!(settings, before);
}

#[test]
fn xdg_character_path_uses_absolute_config_or_home_fallback() {
    let home = std::path::Path::new("/home/example");
    for value in [None, Some(PathBuf::new()), Some(PathBuf::from("relative"))] {
        assert_eq!(
            xdg_path(value, home, ".config").join("characters.json"),
            PathBuf::from("/home/example/.config/voxmoviemaker/characters.json")
        );
    }
    assert_eq!(
        xdg_path(Some(PathBuf::from("/custom")), home, ".config").join("characters.json"),
        PathBuf::from("/custom/voxmoviemaker/characters.json")
    );
}
