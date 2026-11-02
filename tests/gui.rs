use eframe::egui::{self, Event, PointerButton};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    sync::Arc,
    thread,
    time::{Duration, Instant},
};
use voxmoviemaker::{
    app::{App, PathMode},
    assets::Storage,
    media::ffmpeg::Capabilities,
    project::*,
    settings::{Settings, Theme},
    ui::timeline::{LABEL_WIDTH, ROW_HEIGHT, RULER_HEIGHT},
};
struct Fixture {
    root: PathBuf,
    app: App,
    ctx: egui::Context,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("vmm-gui-{}", Id::new_v4()));
        fs::create_dir(&root).unwrap();
        let ctx = egui::Context::default();
        let mut fonts = egui::FontDefinitions::empty();
        let font = [
            "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        ]
        .iter()
        .find_map(|p| fs::read(p).ok())
        .expect("a system font is required for GUI tests");
        fonts
            .font_data
            .insert("system".into(), Arc::new(egui::FontData::from_owned(font)));
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            fonts
                .families
                .entry(family)
                .or_default()
                .push("system".into());
        }
        ctx.set_fonts(fonts);
        let storage = Storage {
            data: root.join("data"),
            cache: root.join("cache"),
            config: root.join("config"),
        };
        let app = App::from_context(ctx.clone(), storage, Settings::default(), None);
        Self { root, app, ctx }
    }
    fn frame(&mut self, events: Vec<Event>) -> egui::FullOutput {
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1200.0, 850.0),
            )),
            events,
            ..Default::default()
        };
        self.ctx.run(raw, |ctx| self.app.show(ctx))
    }
    fn button(position: egui::Pos2, pressed: bool) -> Event {
        Event::PointerButton {
            pos: position,
            button: PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        }
    }
    fn register_character(&mut self, name: &str, speed: f64, style: u32) -> Id {
        use voxmoviemaker::{
            characters::Character,
            voicevox::{Connection, Speaker, Style},
        };
        let mut character =
            Character::new(name.into(), self.app.project.width, self.app.project.height);
        character.voice.speaker_uuid = "speaker-uuid".into();
        character.voice.speaker_name = "speaker".into();
        character.voice.style_name = "Normal".into();
        character.voice.speaker = style;
        character.voice.speed = speed;
        let id = character.id;
        let mut library = self.app.characters.library.clone();
        library.characters.push(character);
        self.app.characters.save(library).unwrap();
        self.app.settings.character = Some(id);
        self.app.connection = Some(Connection {
            version: "test".into(),
            speakers: vec![Speaker {
                name: "speaker".into(),
                speaker_uuid: "speaker-uuid".into(),
                styles: vec![Style {
                    id: style,
                    name: "Normal".into(),
                    style_type: None,
                }],
            }],
        });
        id
    }
    fn image_clip(&mut self, track: usize) -> Id {
        let asset = MediaAsset {
            id: Id::new_v4(),
            name: "image".into(),
            location: AssetLocation::External(self.root.join("image.png")),
            kind: MediaKind::Image,
            duration_seconds: 0.0,
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
        let clip = Clip {
            character: None,
            id: Id::new_v4(),
            source: ClipSource::Media { asset_id: asset.id },
            timeline_start: 0,
            duration: 150,
            source_offset: 0,
            volume: 1.0,
            muted: false,
            effects: Default::default(),
        };
        let id = clip.id;
        self.app.project.assets.push(asset);
        self.app.project.tracks[track].clips.push(clip);
        id
    }
    fn open_clip_menu(&mut self, track: usize) -> egui::FullOutput {
        self.frame(vec![]);
        self.frame(vec![]);
        let canvas = self.app.timeline_rect.unwrap();
        let position = egui::pos2(
            canvas.left() + LABEL_WIDTH + 35.0,
            canvas.top() + RULER_HEIGHT + ROW_HEIGHT * (track as f32 + 0.5),
        );
        self.frame(vec![Event::PointerMoved(position)]);
        for pressed in [true, false] {
            self.frame(vec![Event::PointerButton {
                pos: position,
                button: PointerButton::Secondary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            }]);
        }
        self.frame(vec![])
    }
    fn click_menu(&mut self, output: &egui::FullOutput, key: &str) {
        let label = self.app.tr(key);
        let position = output
            .shapes
            .iter()
            .find_map(|shape| text_position(&shape.shape, &label))
            .unwrap_or_else(|| panic!("menu item missing: {label}"));
        self.frame(vec![Event::PointerMoved(position)]);
        self.frame(vec![Self::button(position, true)]);
        self.frame(vec![Self::button(position, false)]);
    }
    fn wait_for_browser_file(&mut self, name: &str) -> egui::FullOutput {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let output = self.frame(vec![]);
            if output
                .shapes
                .iter()
                .any(|s| text_position(&s.shape, name).is_some())
            {
                // Let the window settle after the asynchronous listing changes its size.
                self.frame(vec![]);
                return self.frame(vec![]);
            }
            assert!(Instant::now() < deadline, "browser listing timed out");
            thread::sleep(Duration::from_millis(5));
        }
    }
    fn prepare_transport(&mut self) -> Id {
        let id = self.image_clip(0);
        fs::write(self.root.join("image.png"), b"mock input").unwrap();
        let binary = self.root.join("mock-ffmpeg");
        fs::write(&binary, "#!/bin/sh\nframes=150\nprevious=\nfor last do\nif [ \"$previous\" = -frames:v ]; then frames=$last; fi\nprevious=$last\ndone\ncase \"$last\" in\npipe:1) exec head -c \"$((frames * 160 * 90 * 4))\" /dev/zero;;\n*) printf mock > \"$last\"; printf 'out_time_us=5000000\\n';;\nesac\n").unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
        self.app.settings.ffmpeg = binary;
        self.app.settings.preview_width = 160;
        self.app.settings.preview_audio = false;
        self.app.tools = Some(Capabilities {
            encoders: " V..... mpeg4\n A..... pcm_s16le".into(),
            formats: " E nut".into(),
            ..Default::default()
        });
        self.app.preview_dirty = false;
        id
    }
    fn wait_for_session(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while self.app.transport.session.is_none() && Instant::now() < deadline {
            self.frame(vec![]);
            thread::sleep(Duration::from_millis(5));
        }
        assert!(self.app.transport.session.is_some(), "{}", self.app.status);
    }
}
fn text_position(shape: &egui::epaint::Shape, label: &str) -> Option<egui::Pos2> {
    match shape {
        egui::epaint::Shape::Text(text) if text.galley.text() == label => {
            Some(text.pos + text.galley.size() / 2.0)
        }
        egui::epaint::Shape::Vec(shapes) => shapes.iter().find_map(|s| text_position(s, label)),
        _ => None,
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
#[test]
fn native_ui_layout_language_theme_and_voice_selection_restore() {
    let mut f = Fixture::new();
    f.frame(vec![]);
    f.frame(vec![]);
    let timeline = f.app.timeline_rect.unwrap();
    assert!(timeline.top() > 200.0);
    assert!(timeline.bottom() < f.app.voice_input_rect.unwrap().top());
    f.app.settings.theme = Theme::Light;
    f.app.apply_appearance(&f.ctx);
    f.app.settings.language = "en-US".into();
    f.app.i18n.language = "en-US".into();
    f.frame(vec![]);
    assert_eq!(
        f.ctx.options(|o| o.theme_preference),
        egui::ThemePreference::Light
    );
    assert_eq!(f.app.tr("properties"), "Properties");
    let a = MediaAsset {
        id: Id::new_v4(),
        name: "voice".into(),
        location: AssetLocation::Voice(Id::new_v4()),
        kind: MediaKind::Audio,
        duration_seconds: 1.0,
        width: 0,
        height: 0,
        has_audio: true,
        sample_rate: Some(48000),
        frame_rate: None,
        codec: "pcm".into(),
        start_time: 0.0,
        rotation: 0,
        video_stream: None,
        audio_stream: Some(0),
    };
    let c = Clip {
        character: None,
        id: Id::new_v4(),
        source: ClipSource::Voicevox {
            asset_id: a.id,
            text: "日本語テキスト".into(),
            settings: VoicevoxSettings {
                speed: 1.2,
                ..Default::default()
            }
            .into(),
            subtitle: None,
        },
        timeline_start: 0,
        duration: 30,
        source_offset: 0,
        volume: 1.0,
        muted: false,
        effects: Default::default(),
    };
    let id = c.id;
    f.app.project.assets.push(a);
    f.app.project.tracks[0].clips.push(c);
    f.app.select(id);
    f.frame(vec![]);
    assert_eq!(f.app.voice_text, "日本語テキスト");
    assert_eq!(f.app.voice_settings.speed, 1.2);
    f.app.request_path(PathMode::Open);
    assert_eq!(f.app.confirm_discard, Some(PathMode::Open));
    assert!(f.app.dirty());
}
#[test]
fn pointer_drag_moves_clip_to_another_generic_layer() {
    let mut f = Fixture::new();
    let a = MediaAsset {
        id: Id::new_v4(),
        name: "image".into(),
        location: AssetLocation::External(f.root.join("image.png")),
        kind: MediaKind::Image,
        duration_seconds: 0.0,
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
    let c = Clip {
        character: None,
        id: Id::new_v4(),
        source: ClipSource::Media { asset_id: a.id },
        timeline_start: 0,
        duration: 150,
        source_offset: 0,
        volume: 1.0,
        muted: false,
        effects: Default::default(),
    };
    let id = c.id;
    f.app.project.assets.push(a);
    f.app.project.tracks[0].clips.push(c);
    f.frame(vec![]);
    f.frame(vec![]);
    let canvas = f.app.timeline_rect.unwrap();
    let start = egui::pos2(
        canvas.left() + LABEL_WIDTH + 35.0,
        canvas.top() + RULER_HEIGHT + ROW_HEIGHT / 2.0,
    );
    f.frame(vec![Event::PointerMoved(start)]);
    f.frame(vec![Fixture::button(start, true)]);
    let end = start + egui::vec2(140.0, ROW_HEIGHT);
    f.frame(vec![Event::PointerMoved(end)]);
    f.frame(vec![Fixture::button(end, false)]);
    assert_eq!(f.app.project.track_of(id), Some(1));
    assert_eq!(f.app.project.clip(id).unwrap().timeline_start, 60);
    f.app.history.undo(&mut f.app.project);
    assert_eq!(f.app.project.track_of(id), Some(0));
}

#[test]
fn context_menu_splits_the_right_clicked_clip_and_supports_undo() {
    let mut f = Fixture::new();
    let previous = f.image_clip(0);
    let target = f.image_clip(1);
    f.app.select(previous);
    f.app.playhead = 75;
    let output = f.open_clip_menu(1);
    assert_eq!(f.app.selected, Some(target));
    assert_eq!(f.app.playhead, 75);
    f.click_menu(&output, "split");
    assert_eq!(f.app.project.clip(previous).unwrap().duration, 150);
    assert_eq!(f.app.project.clip(target).unwrap().duration, 75);
    let clips = &f.app.project.tracks[1].clips;
    assert_eq!(clips.len(), 2);
    let second = clips.iter().find(|c| c.id != target).unwrap();
    assert_eq!(
        (second.timeline_start, second.source_offset, second.duration),
        (75, 75, 75)
    );
    f.app.history.undo(&mut f.app.project);
    assert_eq!(f.app.project.tracks[1].clips.len(), 1);
    assert_eq!(f.app.project.clip(target).unwrap().duration, 150);
}

#[test]
fn context_menu_deletes_its_target_respects_locks_and_supports_undo() {
    for locked in [true, false] {
        let mut f = Fixture::new();
        let previous = f.image_clip(0);
        let target = f.image_clip(1);
        f.app.project.tracks[1].locked = locked;
        f.app.select(previous);
        let output = f.open_clip_menu(1);
        f.click_menu(&output, "delete");
        assert!(f.app.project.clip(previous).is_some());
        assert_eq!(f.app.project.clip(target).is_some(), locked);
        if !locked {
            f.app.history.undo(&mut f.app.project);
            assert!(f.app.project.clip(target).is_some());
        }
    }
}

#[test]
fn context_menu_disables_split_on_boundaries_outside_clip_and_locked_layers() {
    for (frame, locked) in [(0, false), (150, false), (200, false), (75, true)] {
        let mut f = Fixture::new();
        let target = f.image_clip(0);
        f.app.project.tracks[0].locked = locked;
        f.app.playhead = frame;
        let output = f.open_clip_menu(0);
        f.click_menu(&output, "split");
        assert_eq!(f.app.project.tracks[0].clips.len(), 1);
        assert_eq!(f.app.project.clip(target).unwrap().duration, 150);
    }
}

#[test]
fn preview_centers_placeholder_and_portrait_or_landscape_frames() {
    let mut f = Fixture::new();
    f.app.project.width = 240;
    f.app.project.height = 480;
    f.frame(vec![]);
    let output = f.frame(vec![]);
    let placeholder = output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::epaint::Shape::Rect(rect) if rect.fill == egui::Color32::BLACK => Some(rect.rect),
            _ => None,
        })
        .expect("preview placeholder");
    let expected_x = (1200.0 - f.app.settings.properties_width) / 2.0;
    assert!((placeholder.center().x - expected_x).abs() < 1.0);
    for size in [[64, 128], [400, 100]] {
        let texture = f.ctx.load_texture(
            "preview-test",
            egui::ColorImage::filled(size, egui::Color32::RED),
            Default::default(),
        );
        let id = texture.id();
        f.app.texture = Some(texture);
        let output = f.frame(vec![]);
        let rect = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::epaint::Shape::Rect(rect)
                    if rect
                        .brush
                        .as_ref()
                        .is_some_and(|brush| brush.fill_texture_id == id) =>
                {
                    Some(rect.rect)
                }
                _ => None,
            })
            .expect("preview image");
        assert!(rect.center().distance(placeholder.center()) < 1.0);
        // Image painting rounds its corners to device pixels.
        assert!((rect.width() - rect.height() * size[0] as f32 / size[1] as f32).abs() <= 3.0);
    }
}

fn space(pressed: bool) -> Event {
    Event::Key {
        key: egui::Key::Space,
        physical_key: None,
        pressed,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }
}

fn enter(modifiers: egui::Modifiers) -> Event {
    Event::Key {
        key: egui::Key::Enter,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    }
}
fn release_enter() -> Event {
    Event::Key {
        key: egui::Key::Enter,
        physical_key: None,
        pressed: false,
        repeat: false,
        modifiers: Default::default(),
    }
}
fn focus_voice_input(f: &mut Fixture) {
    f.frame(vec![]);
    let point = f.app.voice_input_rect.unwrap().center();
    f.frame(vec![
        Event::PointerMoved(point),
        Fixture::button(point, true),
    ]);
    f.frame(vec![Fixture::button(point, false)]);
}

#[test]
fn character_selection_replaces_speaker_ui_and_preserves_text_and_existing_clips() {
    for select_image in [false, true] {
        let mut f = Fixture::new();
        let first = f.register_character("First character", 1.2, 0);
        let second = f.register_character("Second character", 0.9, 0);
        f.app.settings.character = Some(first);
        if select_image {
            let id = f.image_clip(0);
            f.app.select(id);
        }
        f.app.voice_text = "Keep this text".into();
        let original = f.app.project.clone();
        f.frame(vec![]);
        let output = f.frame(vec![]);
        let selector = output
            .shapes
            .iter()
            .find_map(|s| text_position(&s.shape, "First character  ▾"))
            .unwrap();
        assert!(selector.y > f.app.timeline_rect.unwrap().bottom());
        assert!(selector.y < f.app.voice_input_rect.unwrap().top());
        assert!(
            output
                .shapes
                .iter()
                .all(|s| text_position(&s.shape, "speaker / Normal").is_none())
        );
        f.frame(vec![Event::PointerMoved(selector)]);
        f.frame(vec![Fixture::button(selector, true)]);
        f.frame(vec![Fixture::button(selector, false)]);
        f.frame(vec![]);
        f.frame(vec![Event::Text("second".into())]);
        let output = f.frame(vec![]);
        let option = output
            .shapes
            .iter()
            .find_map(|s| text_position(&s.shape, "Second character"))
            .unwrap();
        f.frame(vec![Event::PointerMoved(option)]);
        f.frame(vec![Fixture::button(option, true)]);
        f.frame(vec![Fixture::button(option, false)]);
        assert_eq!(f.app.settings.character, Some(second));
        assert_eq!(f.app.voice_text, "Keep this text");
        assert_eq!(f.app.project, original);
        assert_eq!(
            f.app.characters.library.get(second).unwrap().voice.speed,
            0.9
        );
    }
}

#[test]
fn character_name_can_be_cleared_and_retyped_without_losing_focus() {
    let mut f = Fixture::new();
    let id = f.register_character("Original", 1.0, 0);
    let focus = focus_character_name(&mut f, id);
    select_all_text(&mut f);
    f.frame(vec![Event::Key {
        key: egui::Key::Backspace,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Default::default(),
    }]);
    f.frame(vec![]);
    assert_eq!(f.ctx.memory(|m| m.focused()), Some(focus));
    assert_eq!(f.app.character_ui.draft.as_ref().unwrap().name, "");
    assert_eq!(f.app.characters.library.get(id).unwrap().name, "Original");
    f.frame(vec![Event::Text("新しい名前".into())]);
    f.frame(vec![]);
    assert_eq!(f.ctx.memory(|m| m.focused()), Some(focus));
    assert_eq!(f.app.characters.library.get(id).unwrap().name, "新しい名前");
}

#[test]
fn character_name_ime_preedit_is_not_saved_and_confirmation_keeps_focus() {
    let mut f = Fixture::new();
    let id = f.register_character("Original", 1.0, 0);
    let focus = focus_character_name(&mut f, id);
    select_all_text(&mut f);
    let path = f.app.storage.config.join("characters.json");
    let original = fs::read(&path).unwrap();
    f.frame(vec![
        Event::Ime(egui::ImeEvent::Enabled),
        Event::Ime(egui::ImeEvent::Preedit("あおい".into())),
    ]);
    f.frame(vec![]);
    assert_eq!(
        fs::read(&path).unwrap(),
        original,
        "preedit must stay in the editor"
    );
    f.frame(vec![enter(egui::Modifiers::NONE)]);
    assert_eq!(f.ctx.memory(|m| m.focused()), Some(focus));
    assert_eq!(f.app.characters.library.get(id).unwrap().name, "Original");
    f.frame(vec![
        Event::Ime(egui::ImeEvent::Commit("葵".into())),
        release_enter(),
        enter(egui::Modifiers::NONE),
    ]);
    f.frame(vec![]);
    assert_eq!(f.ctx.memory(|m| m.focused()), Some(focus));
    assert_eq!(f.app.characters.library.get(id).unwrap().name, "葵");
    f.frame(vec![Event::Text("さん".into())]);
    assert_eq!(f.app.characters.library.get(id).unwrap().name, "葵さん");
}

#[test]
fn character_name_keeps_focus_and_draft_when_a_save_error_appears() {
    let mut f = Fixture::new();
    let id = f.register_character("Original", 1.0, 0);
    let focus = focus_character_name(&mut f, id);
    select_all_text(&mut f);
    let path = f.app.storage.config.join("characters.json");
    let external = format!("{}\n", fs::read_to_string(&path).unwrap());
    fs::write(&path, &external).unwrap();
    f.frame(vec![Event::Text("First".into())]);
    f.frame(vec![]);
    assert_eq!(f.ctx.memory(|m| m.focused()), Some(focus));
    f.frame(vec![Event::Text("-second".into())]);
    assert_eq!(
        f.app.character_ui.draft.as_ref().unwrap().name,
        "First-second"
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), external);
}

fn focus_character_name(f: &mut Fixture, id: Id) -> egui::Id {
    f.app.character_ui.open = true;
    f.app.character_ui.selected = Some(id);
    let label = f.app.characters.library.get(id).unwrap().name.clone();
    f.frame(vec![]);
    let output = f.frame(vec![]);
    let pos = output
        .shapes
        .iter()
        .find_map(|s| text_position(&s.shape, &label))
        .unwrap();
    f.frame(vec![Event::PointerMoved(pos)]);
    f.frame(vec![Fixture::button(pos, true)]);
    f.frame(vec![Fixture::button(pos, false)]);
    f.ctx
        .memory(|m| m.focused())
        .expect("name editor must have focus")
}

fn select_all_text(f: &mut Fixture) {
    for pressed in [true, false] {
        f.frame(vec![Event::Key {
            key: egui::Key::A,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: egui::Modifiers::CTRL | egui::Modifiers::COMMAND,
        }]);
    }
}

#[test]
fn character_manager_creates_renames_duplicates_confirms_deletion_and_reloads() {
    let mut f = Fixture::new();
    f.app.character_ui.open = true;
    f.frame(vec![]);
    let output = f.frame(vec![]);
    f.click_menu(&output, "character-new");
    assert_eq!(f.app.characters.library.characters.len(), 1);
    assert!(f.app.storage.config.join("characters.json").is_file());
    assert!(!f.app.dirty());
    let id = f.app.characters.library.characters[0].id;
    let output = f.frame(vec![]);
    let pos = output
        .shapes
        .iter()
        .find_map(|s| text_position(&s.shape, &f.app.tr("character-new-name")))
        .expect("character name editor");
    f.frame(vec![Event::PointerMoved(pos)]);
    f.frame(vec![Fixture::button(pos, true)]);
    f.frame(vec![Fixture::button(pos, false)]);
    f.frame(vec![Event::Key {
        key: egui::Key::A,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::CTRL | egui::Modifiers::COMMAND,
    }]);
    f.frame(vec![Event::Text("Renamed character".into())]);
    f.frame(vec![]);
    assert_eq!(
        f.app.characters.library.get(id).unwrap().name,
        "Renamed character"
    );
    let output = f.frame(vec![]);
    f.click_menu(&output, "character-duplicate");
    assert_eq!(f.app.characters.library.characters.len(), 2);
    let copy = f.app.character_ui.selected.unwrap();
    assert_ne!(id, copy);
    assert_eq!(
        f.app.characters.library.get(id).unwrap().voice,
        f.app.characters.library.get(copy).unwrap().voice
    );
    let output = f.frame(vec![]);
    f.click_menu(&output, "delete");
    assert_eq!(
        f.app.characters.library.characters.len(),
        2,
        "deletion requires confirmation"
    );
    let output = f.frame(vec![]);
    assert!(
        output.shapes.iter().any(|s| text_position(
            &s.shape,
            &f.app.tr("character-delete-warning")
        )
        .is_some())
    );
    let pos = output
        .shapes
        .iter()
        .filter_map(|s| text_position(&s.shape, &f.app.tr("delete")))
        .next_back()
        .unwrap();
    f.frame(vec![Event::PointerMoved(pos)]);
    f.frame(vec![Fixture::button(pos, true)]);
    f.frame(vec![Fixture::button(pos, false)]);
    assert!(f.app.characters.library.get(copy).is_none());
    assert!(f.app.characters.library.get(id).is_some());
    f.app.new_project();
    assert_eq!(
        f.app.characters.library.get(id).unwrap().name,
        "Renamed character"
    );
    let restarted = App::from_context(
        f.ctx.clone(),
        f.app.storage.clone(),
        Settings::default(),
        None,
    );
    assert_eq!(restarted.characters.library, f.app.characters.library);
}

#[test]
fn missing_or_unselected_characters_and_unavailable_styles_never_start_generation() {
    let mut f = Fixture::new();
    f.app.voice_text = "Keep this phrase".into();
    f.app.generate(false);
    assert!(!f.app.generating);
    assert_eq!(f.app.status, f.app.tr("character-select-first"));
    let id = f.register_character("Narrator", 1.0, 0);
    f.app.connection = None;
    f.app.generate(false);
    assert!(!f.app.generating);
    assert_eq!(f.app.status, f.app.tr("voicevox-offline"));
    f.app.connection = Some(voxmoviemaker::voicevox::Connection {
        version: "test".into(),
        speakers: vec![],
    });
    f.app.generate(false);
    assert!(!f.app.generating);
    assert_eq!(f.app.status, f.app.tr("character-voice-missing"));
    f.app.save_characters(Default::default()).unwrap();
    assert_eq!(f.app.settings.character, Some(id));
    f.app.generate(false);
    assert!(!f.app.generating);
    assert_eq!(f.app.status, f.app.tr("character-missing"));
    assert_eq!(f.app.voice_text, "Keep this phrase");
    assert!(f.app.project.assets.is_empty());
    assert!(!f.app.storage.data.exists());
    f.frame(vec![]);
}

#[test]
fn character_speaker_changes_only_offer_matching_styles_and_update_the_style_id() {
    use voxmoviemaker::voicevox::{Speaker, Style};
    let mut f = Fixture::new();
    let id = f.register_character("Profile", 1.0, 0);
    f.app.connection.as_mut().unwrap().speakers.push(Speaker {
        name: "Other speaker".into(),
        speaker_uuid: "other-uuid".into(),
        styles: vec![
            Style {
                id: 7,
                name: "Other style A".into(),
                style_type: None,
            },
            Style {
                id: 8,
                name: "Other style B".into(),
                style_type: None,
            },
        ],
    });
    f.app.character_ui.open = true;
    f.app.character_ui.selected = Some(id);
    f.frame(vec![]);
    let output = f.frame(vec![]);
    let pos = output
        .shapes
        .iter()
        .find_map(|s| text_position(&s.shape, "Normal"))
        .unwrap();
    f.frame(vec![Event::PointerMoved(pos)]);
    f.frame(vec![Fixture::button(pos, true)]);
    f.frame(vec![Fixture::button(pos, false)]);
    let output = f.frame(vec![]);
    assert!(
        output
            .shapes
            .iter()
            .all(|s| text_position(&s.shape, "Other style A").is_none())
    );
    f.frame(vec![Event::Key {
        key: egui::Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Default::default(),
    }]);
    let output = f.frame(vec![]);
    let pos = output
        .shapes
        .iter()
        .find_map(|s| text_position(&s.shape, "speaker"))
        .unwrap();
    f.frame(vec![Event::PointerMoved(pos)]);
    f.frame(vec![Fixture::button(pos, true)]);
    f.frame(vec![Fixture::button(pos, false)]);
    let output = f.frame(vec![]);
    let pos = output
        .shapes
        .iter()
        .find_map(|s| text_position(&s.shape, "Other speaker"))
        .unwrap();
    f.frame(vec![Event::PointerMoved(pos)]);
    f.frame(vec![Fixture::button(pos, true)]);
    f.frame(vec![Fixture::button(pos, false)]);
    let voice = &f.app.characters.library.get(id).unwrap().voice;
    assert_eq!(voice.speaker_uuid, "other-uuid");
    assert_eq!(voice.speaker, 7);
    assert_eq!(voice.style_name, "Other style A");
    use voxmoviemaker::{media::fonts::FontId, ui::searchable_combo_box::Choice};
    let font = FontId {
        path: f.root.join("CharacterFont.ttc"),
        index: 2,
    };
    f.app.fonts.loaded = true;
    f.app.fonts.choices = vec![
        Choice::new(FontId::default(), "Automatic"),
        Choice::new(font.clone(), "Character font face"),
    ];
    f.app.character_ui.tab = 2;
    f.frame(vec![]);
    let output = f.frame(vec![]);
    let label = format!("{}  ▾", f.app.tr("font-automatic"));
    let pos = output
        .shapes
        .iter()
        .find_map(|s| text_position(&s.shape, &label))
        .unwrap();
    f.frame(vec![Event::PointerMoved(pos)]);
    f.frame(vec![Fixture::button(pos, true)]);
    f.frame(vec![Fixture::button(pos, false)]);
    f.frame(vec![Event::Text("character font".into())]);
    let output = f.frame(vec![]);
    let pos = output
        .shapes
        .iter()
        .find_map(|s| text_position(&s.shape, "Character font face"))
        .unwrap();
    f.frame(vec![Event::PointerMoved(pos)]);
    f.frame(vec![Fixture::button(pos, true)]);
    f.frame(vec![Fixture::button(pos, false)]);
    let profile = f.app.characters.library.get(id).unwrap();
    assert_eq!(profile.subtitle.font, font.path);
    assert_eq!(profile.subtitle.font_index, 2);
    assert_eq!(profile.voice.speaker, 7);
    assert!(
        !f.app.dirty(),
        "global font edits must not modify the project"
    );
}

#[test]
fn changing_clip_character_keeps_overrides_and_reset_controls_restore_inheritance() {
    let mut f = Fixture::new();
    let first = f.register_character("First", 1.0, 0);
    let second = f.register_character("Second", 0.9, 0);
    f.app.insert_text(false, false);
    let target = f.app.selected.unwrap();
    let mut binding = voxmoviemaker::characters::CharacterBinding::new(first);
    binding.voice.speed = Some(1.5);
    binding.subtitle.size = Some(72.0);
    binding.subtitle.x = Some(500.0);
    f.app.project.clip_mut(target).unwrap().character = Some(Box::new(binding.clone()));
    f.frame(vec![]);
    let output = f.frame(vec![]);
    let pos = output
        .shapes
        .iter()
        .find_map(|s| text_position(&s.shape, "First  ▾"))
        .unwrap();
    f.frame(vec![Event::PointerMoved(pos)]);
    f.frame(vec![Fixture::button(pos, true)]);
    f.frame(vec![Fixture::button(pos, false)]);
    let output = f.frame(vec![]);
    let pos = output
        .shapes
        .iter()
        .find_map(|s| text_position(&s.shape, "Second"))
        .unwrap();
    f.frame(vec![Event::PointerMoved(pos)]);
    f.frame(vec![Fixture::button(pos, true)]);
    f.frame(vec![Fixture::button(pos, false)]);
    let changed = f
        .app
        .project
        .clip(target)
        .unwrap()
        .character
        .as_ref()
        .unwrap();
    assert_eq!(changed.character_id, second);
    assert_eq!(changed.voice, binding.voice);
    assert_eq!(changed.subtitle, binding.subtitle);
    let output = f.frame(vec![]);
    f.click_menu(&output, "override-reset");
    assert_eq!(
        f.app
            .project
            .clip(target)
            .unwrap()
            .character
            .as_ref()
            .unwrap()
            .subtitle
            .x,
        None
    );
    let output = f.frame(vec![]);
    f.click_menu(&output, "overrides-reset-all");
    let changed = f
        .app
        .project
        .clip(target)
        .unwrap()
        .character
        .as_ref()
        .unwrap();
    assert_eq!(changed.voice, Default::default());
    assert_eq!(changed.subtitle, Default::default());
    f.app.finish_property_edit();
    f.app.history.undo(&mut f.app.project);
    assert_eq!(
        f.app
            .project
            .clip(target)
            .unwrap()
            .character
            .as_ref()
            .unwrap()
            .subtitle
            .size,
        Some(72.0)
    );
}

#[test]
fn searchable_font_selection_applies_path_and_collection_index_to_selected_text() {
    use voxmoviemaker::{media::fonts::FontId, ui::searchable_combo_box::Choice};
    let mut f = Fixture::new();
    let font = FontId {
        path: f.root.join("日本語フォント.ttc"),
        index: 2,
    };
    f.app.fonts.loaded = true;
    f.app.fonts.choices = vec![
        Choice::new(FontId::default(), "Automatic"),
        Choice::new(font.clone(), "Collection face"),
    ];
    f.app.insert_text(false, false);
    let target = f.app.selected.unwrap();
    let original = f.app.project.clone();
    f.frame(vec![]);
    let output = f.frame(vec![]);
    let label = format!("{}  ▾", f.app.tr("font-automatic"));
    let pos = output
        .shapes
        .iter()
        .find_map(|s| text_position(&s.shape, &label))
        .expect("selected text must expose a searchable font selector");
    f.frame(vec![Event::PointerMoved(pos)]);
    f.frame(vec![Fixture::button(pos, true)]);
    f.frame(vec![Fixture::button(pos, false)]);
    f.frame(vec![]);
    f.frame(vec![Event::Text("cOlLeCtIoN".into())]);
    let output = f.frame(vec![]);
    assert_eq!(
        f.app.project, original,
        "searching must not edit the project"
    );
    let pos = output
        .shapes
        .iter()
        .find_map(|s| text_position(&s.shape, "Collection face"))
        .expect("font names must be searchable");
    f.frame(vec![Event::PointerMoved(pos)]);
    f.frame(vec![Fixture::button(pos, true)]);
    f.frame(vec![Fixture::button(pos, false)]);
    let ClipSource::Text { settings } = &f.app.project.clip(target).unwrap().source else {
        panic!("expected text clip");
    };
    assert_eq!(settings.font, font.path);
    assert_eq!(settings.font_index, 2);
    assert!(!f.app.fonts.search.is_open());
    f.app.finish_property_edit();
    f.app.history.undo(&mut f.app.project);
    assert_eq!(f.app.project, original);
}

#[test]
fn voice_input_starts_on_one_row_and_shift_enter_adds_rows() {
    let mut f = Fixture::new();
    focus_voice_input(&mut f);
    let height = f.app.voice_input_rect.unwrap().height();
    assert!(height < 35.0);
    f.frame(vec![
        Event::Text("first".into()),
        enter(egui::Modifiers::SHIFT),
    ]);
    assert_eq!(f.app.voice_text, "first\n");
    f.frame(vec![Event::Text("second".into())]);
    f.frame(vec![]);
    assert!(f.app.voice_input_rect.unwrap().height() > height + 8.0);
    // Enter must neither insert a newline nor enqueue another job while busy.
    f.app.generating = true;
    f.frame(vec![enter(egui::Modifiers::NONE)]);
    assert_eq!(f.app.voice_text, "first\nsecond");
}

#[test]
fn voice_ime_confirmation_does_not_submit_synthesis() {
    let mut f = Fixture::new();
    f.register_character("Narrator", 1.0, 0);
    f.app.settings.voicevox_url = "invalid-url".into();
    focus_voice_input(&mut f);
    f.frame(vec![enter(egui::Modifiers::NONE)]);
    assert!(!f.app.generating);
    f.frame(vec![
        Event::Ime(egui::ImeEvent::Enabled),
        Event::Ime(egui::ImeEvent::Preedit("にほん".into())),
        enter(egui::Modifiers::NONE),
    ]);
    assert!(!f.app.generating);
    f.frame(vec![
        Event::Ime(egui::ImeEvent::Commit("日本".into())),
        enter(egui::Modifiers::NONE),
    ]);
    assert!(!f.app.generating);
    assert_eq!(f.app.voice_text, "日本");
    f.frame(vec![release_enter(), enter(egui::Modifiers::NONE)]);
    assert!(f.app.generating);
    assert!(f.app.voice_text.is_empty());
}

#[test]
fn consecutive_enter_synthesis_creates_new_clips_and_only_explicit_regeneration_replaces() {
    use std::{
        io::{Read, Write},
        net::TcpListener,
    };
    let mut f = Fixture::new();
    let first_character = f.register_character("First narrator", 1.2, 0);
    let second_character = f.register_character("Second narrator", 0.9, 0);
    f.app.settings.character = Some(first_character);
    f.app.voice_settings.speed = 2.0; // Legacy state must not drive character synthesis.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    f.app.settings.voicevox_url = format!("http://{}", listener.local_addr().unwrap());
    let mut bytes = std::io::Cursor::new(Vec::new());
    {
        let mut wav = hound::WavWriter::new(
            &mut bytes,
            hound::WavSpec {
                channels: 1,
                sample_rate: 48000,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            },
        )
        .unwrap();
        for _ in 0..4800 {
            wav.write_sample(0i16).unwrap();
        }
        wav.finalize().unwrap();
    }
    let audio = bytes.into_inner();
    let server = thread::spawn(move || {
        for index in 0..6 {
            let deadline = Instant::now() + Duration::from_secs(5);
            let mut stream = loop {
                if let Ok((stream, _)) = listener.accept() {
                    break stream;
                }
                assert!(
                    Instant::now() < deadline,
                    "missing VOICEVOX request {index}"
                );
                thread::sleep(Duration::from_millis(2));
            };
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut request = Vec::new();
            let mut byte = [0];
            while !request.ends_with(b"\r\n\r\n") {
                stream.read_exact(&mut byte).unwrap();
                request.push(byte[0]);
            }
            let header = String::from_utf8(request).unwrap();
            let length = header
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .and_then(|n| n.trim().parse::<usize>().ok())
                })
                .unwrap_or(0);
            let mut body = vec![0; length];
            stream.read_exact(&mut body).unwrap();
            let query = index % 2 == 0;
            assert!(header.starts_with(if query {
                "POST /audio_query?"
            } else {
                "POST /synthesis?"
            }));
            assert!(header.contains("speaker=0"));
            if !query {
                let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
                let expected = match index {
                    1 => 1.2,
                    3 => 0.9,
                    _ => 1.6,
                };
                assert_eq!(value["speedScale"], expected);
            }
            if index == 1 {
                thread::sleep(Duration::from_millis(100));
            }
            let body = if query {
                br#"{"accent_phrases":[],"speedScale":1,"outputSamplingRate":48000}"#.as_slice()
            } else {
                audio.as_slice()
            };
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: {}\r\nConnection: close\r\n\r\n", body.len(), if query { "application/json" } else { "audio/wav" }).unwrap();
            stream.write_all(body).unwrap();
        }
    });
    focus_voice_input(&mut f);
    f.frame(vec![
        Event::Text("first".into()),
        enter(egui::Modifiers::NONE),
    ]);
    assert!(f.app.generating);
    assert!(f.app.voice_text.is_empty());
    f.frame(vec![release_enter(), Event::Text("second".into())]);
    let wait = |f: &mut Fixture| {
        let deadline = Instant::now() + Duration::from_secs(4);
        while f.app.generating && Instant::now() < deadline {
            f.frame(vec![]);
            thread::sleep(Duration::from_millis(2));
        }
        assert!(!f.app.generating, "{}", f.app.status);
    };
    f.app.settings.character = Some(second_character);
    wait(&mut f);
    assert_eq!(
        f.app.voice_text, "second",
        "completion must preserve the next draft"
    );
    let first = f.app.project.clip(f.app.selected.unwrap()).unwrap().clone();
    assert_eq!(
        first.character.as_ref().unwrap().character_id,
        first_character
    );
    f.frame(vec![enter(egui::Modifiers::NONE)]);
    assert!(f.app.voice_text.is_empty());
    wait(&mut f);
    assert_eq!(f.app.project.clip(first.id), Some(&first));
    let second = f.app.selected.unwrap();
    assert_ne!(first.id, second);
    assert_eq!(
        f.app
            .project
            .tracks
            .iter()
            .map(|t| t.clips.len())
            .sum::<usize>(),
        2
    );
    assert!(f.app.voice_text.is_empty());
    f.app.select(second);
    assert_eq!(
        f.app.voice_text, "second",
        "explicitly selecting the clip restores its phrase"
    );
    assert_eq!(
        f.app
            .project
            .clip(second)
            .unwrap()
            .character
            .as_ref()
            .unwrap()
            .character_id,
        second_character
    );
    f.app
        .project
        .clip_mut(second)
        .unwrap()
        .character
        .as_mut()
        .unwrap()
        .voice
        .speed = Some(1.6);
    f.app.voice_text = "updated second".into();
    let output = f.frame(vec![]);
    f.click_menu(&output, "regenerate");
    wait(&mut f);
    assert_eq!(f.app.selected, Some(second));
    assert_eq!(f.app.project.clip(first.id), Some(&first));
    assert!(
        matches!(&f.app.project.clip(second).unwrap().source, ClipSource::Voicevox { text, .. } if text == "updated second")
    );
    assert!(f.app.voice_text.is_empty());
    server.join().unwrap();
}

#[test]
fn failed_synthesis_restores_its_phrase_without_erasing_a_new_draft() {
    for draft in ["", "next draft"] {
        let mut f = Fixture::new();
        let before = f.app.project.clone();
        f.app.voice_text = draft.into();
        f.app.generating = true;
        let request = voxmoviemaker::app::VoiceRequest {
            character: None,
            project: f.app.project.project_uuid,
            track: f.app.project.tracks[0].id,
            start: 0,
            replace: None,
            original: None,
            text: "failed phrase".into(),
            settings: Default::default(),
            audition: false,
        };
        f.app
            .worker
            .submit(move || {
                voxmoviemaker::app::Event::Voice(Box::new(request), Err("api-timeout".into()))
            })
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while f.app.generating && Instant::now() < deadline {
            f.frame(vec![]);
            thread::sleep(Duration::from_millis(2));
        }
        assert!(!f.app.generating);
        assert_eq!(f.app.project, before);
        assert_eq!(
            f.app.voice_text,
            if draft.is_empty() {
                "failed phrase"
            } else {
                draft
            }
        );
    }
}

#[test]
fn transport_buttons_pause_resume_seek_stop_and_edit_cancel() {
    let mut f = Fixture::new();
    let clip = f.prepare_transport();
    f.frame(vec![]);
    let output = f.frame(vec![]);
    f.click_menu(&output, "play-timeline");
    f.wait_for_session();
    thread::sleep(Duration::from_millis(80));
    let output = f.frame(vec![]);
    f.click_menu(&output, "pause-timeline");
    assert!(!f.app.transport.active());
    let paused = f.app.playhead;
    thread::sleep(Duration::from_millis(30));
    f.frame(vec![]);
    assert_eq!(f.app.playhead, paused);
    f.frame(vec![space(true)]);
    f.frame(vec![space(false)]);
    f.wait_for_session();
    assert_eq!(f.app.transport.session.as_ref().unwrap().start, paused);
    f.app.seek_timeline(60, &f.ctx.clone());
    f.wait_for_session();
    assert_eq!(f.app.transport.session.as_ref().unwrap().start, 60);
    let output = f.frame(vec![]);
    f.click_menu(&output, "stop-timeline");
    assert_eq!(f.app.playhead, 0);
    assert!(!f.app.transport.active());
    f.app.start_timeline(&f.ctx.clone());
    f.wait_for_session();
    f.app.edit(|project| commands::delete(project, clip));
    assert!(!f.app.transport.active());
    assert!(f.app.project.clip(clip).is_none());
}

#[test]
fn transport_preparation_cancels_and_space_does_not_steal_text_input() {
    let mut f = Fixture::new();
    f.prepare_transport();
    f.frame(vec![]);
    f.app.start_timeline(&f.ctx.clone());
    assert!(f.app.transport.preparing.is_some());
    f.app.pause_timeline();
    assert!(!f.app.transport.active());
    let position = f.app.voice_input_rect.unwrap().center();
    f.frame(vec![Event::PointerMoved(position)]);
    f.frame(vec![Fixture::button(position, true)]);
    f.frame(vec![Fixture::button(position, false)]);
    assert!(f.ctx.wants_keyboard_input());
    f.frame(vec![space(true), Event::Text(" ".into())]);
    assert!(!f.app.transport.active());
    assert_eq!(f.app.voice_text, " ");
}

#[test]
fn subtitle_button_creates_editable_generic_clip_and_paste_needs_no_asset() {
    let mut f = Fixture::new();
    f.frame(vec![]);
    let output = f.frame(vec![]);
    f.click_menu(&output, "add-subtitle");
    let id = f.app.selected.unwrap();
    let clip = f.app.project.clip(id).unwrap().clone();
    assert!(matches!(clip.source, ClipSource::Text { .. }));
    assert!(clip.source.asset_id().is_none());
    assert!(f.app.project.assets.is_empty());
    let output = f.frame(vec![]);
    assert!(
        output
            .shapes
            .iter()
            .any(|shape| text_position(&shape.shape, &f.app.tr("font-select")).is_some())
    );
    f.app.clipboard = Some(clip);
    f.app.playhead = 90;
    f.app.paste();
    assert_eq!(f.app.project.tracks[0].clips.len(), 2);
    f.app.history.undo(&mut f.app.project);
    assert_eq!(f.app.project.tracks[0].clips.len(), 1);
}

#[test]
fn voice_subtitle_stays_in_one_clip_and_respects_locked_layer() {
    let mut f = Fixture::new();
    let id = f.image_clip(0);
    let asset_id = f.app.project.clip(id).unwrap().source.asset_id().unwrap();
    let asset = f
        .app
        .project
        .assets
        .iter_mut()
        .find(|a| a.id == asset_id)
        .unwrap();
    asset.kind = MediaKind::Audio;
    asset.duration_seconds = 5.0;
    asset.has_audio = true;
    f.app.project.clip_mut(id).unwrap().source = ClipSource::Voicevox {
        asset_id,
        text: "日本語のセリフ".into(),
        settings: Default::default(),
        subtitle: None,
    };
    f.app.project.tracks[0].locked = true;
    f.app.i18n.language = "zh-CN".into();
    f.app.select(id);
    f.app.insert_text(true, true);
    assert_eq!(f.app.status, f.app.tr("track-locked"));
    assert!(matches!(
        f.app.project.clip(id).unwrap().source,
        ClipSource::Voicevox { subtitle: None, .. }
    ));
    f.app.project.tracks[0].locked = false;
    f.app.insert_text(true, true);
    let clip = f.app.project.clip(id).unwrap();
    let ClipSource::Voicevox {
        subtitle: Some(style),
        ..
    } = &clip.source
    else {
        panic!()
    };
    assert_eq!(style.text, "日本語のセリフ");
    assert_eq!((clip.timeline_start, clip.duration), (0, 150));
    assert_eq!(
        f.app
            .project
            .tracks
            .iter()
            .map(|t| t.clips.len())
            .sum::<usize>(),
        1
    );
    assert_eq!(f.app.selected, Some(id));
    f.app.history.undo(&mut f.app.project);
    assert!(matches!(
        f.app.project.clip(id).unwrap().source,
        ClipSource::Voicevox { subtitle: None, .. }
    ));
}

#[test]
fn regenerated_voice_keeps_gain_mute_and_clamps_existing_fades() {
    let mut f = Fixture::new();
    let id = f.image_clip(0);
    let asset_id = f.app.project.clip(id).unwrap().source.asset_id().unwrap();
    let asset = f
        .app
        .project
        .assets
        .iter_mut()
        .find(|a| a.id == asset_id)
        .unwrap();
    asset.kind = MediaKind::Audio;
    asset.duration_seconds = 5.0;
    asset.has_audio = true;
    let clip = f.app.project.clip_mut(id).unwrap();
    clip.source = ClipSource::Voicevox {
        asset_id,
        text: "old".into(),
        settings: Default::default(),
        subtitle: Some(TextSettings {
            text: "old".into(),
            size: 36.0,
            color: [255, 255, 0, 255],
            ..Default::default()
        }),
    };
    clip.volume = 0.5;
    clip.muted = true;
    clip.effects.audio_fade_out = 120;
    let original = f.app.project.clip(id).unwrap().clone();
    let mut generated = f.app.project.asset(asset_id).unwrap().clone();
    generated.id = Id::new_v4();
    generated.duration_seconds = 2.0;
    let generated_id = generated.id;
    let request = voxmoviemaker::app::VoiceRequest {
        character: None,
        project: f.app.project.project_uuid,
        track: f.app.project.tracks[0].id,
        start: 0,
        replace: Some(id),
        original: Some(original),
        text: "new".into(),
        settings: Default::default(),
        audition: false,
    };
    f.app
        .worker
        .submit(move || voxmoviemaker::app::Event::Voice(Box::new(request), Ok(generated)))
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while f.app.project.clip(id).unwrap().source.asset_id() != Some(generated_id)
        && Instant::now() < deadline
    {
        f.frame(vec![]);
        thread::sleep(Duration::from_millis(5));
    }
    let clip = f.app.project.clip(id).unwrap();
    assert_eq!(clip.source.asset_id(), Some(generated_id));
    assert_eq!(clip.volume, 0.5);
    assert!(clip.muted);
    assert_eq!(clip.effects.audio_fade_out, 60);
    assert!(f.app.project.asset(asset_id).is_ok());
    let ClipSource::Voicevox {
        subtitle: Some(style),
        ..
    } = &clip.source
    else {
        panic!()
    };
    assert_eq!(style.text, "new");
    assert_eq!(style.size, 36.0);
    assert_eq!(style.color, [255, 255, 0, 255]);
}

#[test]
fn text_button_shows_a_named_timeline_clip_and_supports_normal_edits() {
    let mut f = Fixture::new();
    f.frame(vec![]);
    let output = f.frame(vec![]);
    f.click_menu(&output, "add-text");
    let id = f.app.selected.unwrap();
    let clip = f.app.project.clip(id).unwrap().clone();
    let ClipSource::Text { settings } = &clip.source else {
        panic!()
    };
    assert!(!settings.subtitle);
    assert_eq!(settings.anchor, TextAnchor::Center);
    let title = format!("{} · {}", f.app.tr("text-clip"), settings.text);
    let output = f.frame(vec![]);
    let position = output
        .shapes
        .iter()
        .find_map(|s| text_position(&s.shape, &title))
        .expect("Text must be drawn on the timeline");
    assert!(f.app.timeline_rect.unwrap().contains(position));
    f.app.edit(|p| commands::move_clip(p, id, 1, 30));
    f.app.playhead = 60;
    f.app.split_selected();
    assert_eq!(f.app.project.tracks[1].clips.len(), 2);
    f.app.delete_selected();
    assert_eq!(f.app.project.tracks[1].clips.len(), 1);
    f.app.history.undo(&mut f.app.project);
    assert_eq!(f.app.project.tracks[1].clips.len(), 2);
}

#[test]
fn synthesis_places_one_audio_subtitle_clip_and_failure_keeps_it() {
    let mut f = Fixture::new();
    let asset = MediaAsset {
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
    let request = voxmoviemaker::app::VoiceRequest {
        character: None,
        project: f.app.project.project_uuid,
        track: f.app.project.tracks[2].id,
        start: 30,
        replace: None,
        original: None,
        text: "自動字幕です".into(),
        settings: Default::default(),
        audition: false,
    };
    f.app
        .worker
        .submit(move || voxmoviemaker::app::Event::Voice(Box::new(request), Ok(asset)))
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while f.app.project.tracks[2].clips.is_empty() && Instant::now() < deadline {
        f.frame(vec![]);
        thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        f.app
            .project
            .tracks
            .iter()
            .map(|t| t.clips.len())
            .sum::<usize>(),
        1
    );
    let clip = f.app.project.tracks[2].clips[0].clone();
    let ClipSource::Voicevox {
        subtitle: Some(style),
        ..
    } = &clip.source
    else {
        panic!()
    };
    assert_eq!(style.text, "自動字幕です");
    assert_eq!((clip.timeline_start, clip.duration), (30, 60));
    let plan =
        voxmoviemaker::media::render_plan::RenderPlan::new(&f.app.project, &f.app.storage).unwrap();
    assert_eq!(plan.visual_order(40), vec![clip.id]);
    assert_eq!(plan.clips.iter().filter(|c| c.audio).count(), 1);
    assert_eq!(plan.clips.iter().filter(|c| c.visual).count(), 1);
    let before = f.app.project.clone();
    let request = voxmoviemaker::app::VoiceRequest {
        character: None,
        project: f.app.project.project_uuid,
        track: f.app.project.tracks[2].id,
        start: 0,
        replace: Some(clip.id),
        original: Some(clip.clone()),
        text: "failed".into(),
        settings: Default::default(),
        audition: false,
    };
    f.app.generating = true;
    f.app
        .worker
        .submit(move || {
            voxmoviemaker::app::Event::Voice(Box::new(request), Err("api-timeout".into()))
        })
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while f.app.generating && Instant::now() < deadline {
        f.frame(vec![]);
        thread::sleep(Duration::from_millis(5));
    }
    assert!(!f.app.generating);
    assert_eq!(f.app.project, before);
    f.app.playhead = 60;
    f.app.split_selected();
    assert_eq!(f.app.project.tracks[2].clips.len(), 2);
    for clip in &f.app.project.tracks[2].clips {
        assert!(
            matches!(&clip.source, ClipSource::Voicevox { subtitle: Some(style), .. } if style.text == "自動字幕です")
        );
    }
    f.app.history.undo(&mut f.app.project);
    assert_eq!(f.app.project, before);
}

#[test]
fn file_browser_selects_a_japanese_filename_without_typing_its_path() {
    let mut f = Fixture::new();
    let folder = f.root.join("素材フォルダー");
    fs::create_dir(&folder).unwrap();
    let file = folder.join("日本語の画像.png");
    image::RgbaImage::from_pixel(16, 16, image::Rgba([255, 0, 0, 255]))
        .save(&file)
        .unwrap();
    let ffprobe = f.root.join("mock-ffprobe");
    fs::write(&ffprobe, "#!/bin/sh\nprintf '%s' '{\"streams\":[{\"index\":0,\"codec_type\":\"video\",\"codec_name\":\"png\",\"width\":16,\"height\":16}],\"format\":{\"format_name\":\"png_pipe\"}}'\n").unwrap();
    fs::set_permissions(&ffprobe, fs::Permissions::from_mode(0o700)).unwrap();
    f.app.settings.ffprobe = ffprobe;
    let mut picker = voxmoviemaker::ui::file_picker::FilePicker::built_in(
        PathMode::Import,
        f.app.project.project_uuid,
        f.app.project.tracks[1].id,
        45,
    );
    picker.browse(folder.clone(), &f.ctx);
    f.app.file_picker = Some(picker);
    let deadline = Instant::now() + Duration::from_secs(2);
    let output = loop {
        let output = f.frame(vec![]);
        if output
            .shapes
            .iter()
            .any(|s| text_position(&s.shape, "日本語の画像.png").is_some())
        {
            break output;
        }
        assert!(Instant::now() < deadline, "browser listing timed out");
        thread::sleep(Duration::from_millis(5));
    };
    let pos = output
        .shapes
        .iter()
        .find_map(|s| text_position(&s.shape, "日本語の画像.png"))
        .unwrap();
    f.frame(vec![Event::PointerMoved(pos)]);
    f.frame(vec![Fixture::button(pos, true)]);
    f.frame(vec![Fixture::button(pos, false)]);
    let output = f.frame(vec![]);
    f.app.track = 4;
    f.app.playhead = 300;
    f.click_menu(&output, "select-file");
    let deadline = Instant::now() + Duration::from_secs(2);
    while f.app.project.tracks[1].clips.is_empty() && Instant::now() < deadline {
        f.frame(vec![]);
        thread::sleep(Duration::from_millis(5));
    }
    assert!(f.app.file_picker.is_none());
    assert_eq!(f.app.project.tracks[1].clips.len(), 1, "{}", f.app.status);
    assert_eq!(f.app.project.tracks[1].clips[0].timeline_start, 45);
    let id = f.app.project.tracks[1].clips[0].source.asset_id().unwrap();
    assert_eq!(
        f.app.project.asset(id).unwrap().location,
        AssetLocation::External(file)
    );
}

#[test]
fn export_browser_selects_a_new_japanese_output_and_exports_without_typing_a_path() {
    let mut f = Fixture::new();
    f.prepare_transport();
    let tools = f.app.tools.as_mut().unwrap();
    tools.encoders.push_str("\n V..... libx264\n A..... aac");
    tools.formats.push_str("\n E mp4");
    let before = f.app.project.clone();
    f.app.request_path(PathMode::Export);
    assert!(f.app.path_dialog.as_ref().unwrap().selected_path.is_none());
    let filename = f.root.join("完成した 動画");
    f.app.file_picker = Some(voxmoviemaker::ui::file_picker::FilePicker::built_in_save(
        PathMode::Export,
        f.app.project.project_uuid,
        filename,
    ));
    let output = f.wait_for_browser_file("image.png");
    f.click_menu(&output, "select-export-path");
    let target = f.root.join("完成した 動画.mp4");
    assert!(f.app.file_picker.is_none());
    assert_eq!(
        f.app.path_dialog.as_ref().unwrap().selected_path.as_ref(),
        Some(&target)
    );
    assert!(!target.exists());
    assert!(
        f.app.export.is_none(),
        "choosing a destination alone must not export"
    );
    let output = f.frame(vec![]);
    f.click_menu(&output, "ok");
    let deadline = Instant::now() + Duration::from_secs(3);
    while f.app.export.is_some() && Instant::now() < deadline {
        f.frame(vec![]);
        thread::sleep(Duration::from_millis(5));
    }
    assert!(f.app.export.is_none());
    assert_eq!(f.app.status, f.app.tr("export-complete"));
    assert_eq!(fs::read(target).unwrap(), b"mock");
    assert_eq!(f.app.project, before);
    assert_eq!(fs::read(f.root.join("image.png")).unwrap(), b"mock input");
}

#[test]
fn export_destination_cancellation_and_existing_files_are_protected() {
    let mut f = Fixture::new();
    f.prepare_transport();
    f.app.request_path(PathMode::Export);
    let target = f.root.join("existing.mp4");
    fs::write(&target, b"existing video").unwrap();
    f.app.file_picker = Some(voxmoviemaker::ui::file_picker::FilePicker::built_in_save(
        PathMode::Export,
        f.app.project.project_uuid,
        target.clone(),
    ));
    let output = f.wait_for_browser_file("image.png");
    f.click_menu(&output, "cancel");
    assert!(f.app.file_picker.is_none());
    assert!(f.app.path_dialog.as_ref().unwrap().selected_path.is_none());
    assert!(f.app.export.is_none());
    f.app.file_picker = Some(voxmoviemaker::ui::file_picker::FilePicker::built_in_save(
        PathMode::Export,
        f.app.project.project_uuid,
        target.clone(),
    ));
    let output = f.wait_for_browser_file("image.png");
    f.click_menu(&output, "select-export-path");
    let output = f.frame(vec![]);
    assert!(
        output
            .shapes
            .iter()
            .any(|s| text_position(&s.shape, &f.app.tr("output-exists")).is_some())
    );
    f.click_menu(&output, "ok"); // disabled while the target exists
    assert!(f.app.export.is_none());
    assert!(f.app.path_dialog.is_some());
    assert_eq!(fs::read(&target).unwrap(), b"existing video");
}

#[test]
fn project_save_browser_saves_japanese_path_and_ctrl_s_keeps_destination() {
    use voxmoviemaker::{project::persistence, ui::file_picker::FilePicker};
    let mut f = Fixture::new();
    f.prepare_transport();
    let target = f.root.join("編集した 動画.vmm.json");
    // Use the built-in fallback to exercise the save workflow without opening an OS window.
    f.app.file_picker = Some(FilePicker::built_in_save(
        PathMode::Save,
        f.app.project.project_uuid,
        f.root.join("編集した 動画"),
    ));
    f.app.save_current();
    assert_eq!(f.app.path_dialog.as_ref().unwrap().mode, PathMode::Save);
    let output = f.wait_for_browser_file("image.png");
    f.click_menu(&output, "select-export-path");
    assert!(f.app.file_picker.is_none());
    assert_eq!(
        f.app.path_dialog.as_ref().unwrap().selected_path.as_ref(),
        Some(&target)
    );
    assert!(!target.exists(), "choosing alone must not save");
    let output = f.frame(vec![]);
    f.click_menu(&output, "ok");
    assert_eq!(f.app.project_path.as_ref(), Some(&target));
    assert_eq!(persistence::load(&target).unwrap(), f.app.project);
    assert!(!f.app.dirty());
    f.app.project.tracks[0].name = "変更後".into();
    f.app.save_current();
    assert!(f.app.file_picker.is_none());
    assert!(f.app.path_dialog.is_none());
    assert_eq!(persistence::load(&target).unwrap(), f.app.project);
    assert!(!f.app.dirty());

    let original = fs::read(&target).unwrap();
    let second = f.root.join("別のプロジェクト.vmm.json");
    f.app.project.tracks[0].name = "別名の編集".into();
    f.app.file_picker = Some(FilePicker::built_in_save(
        PathMode::Save,
        f.app.project.project_uuid,
        second.clone(),
    ));
    f.app.request_path(PathMode::Save);
    let output = f.wait_for_browser_file("image.png");
    f.click_menu(&output, "select-export-path");
    let output = f.frame(vec![]);
    f.click_menu(&output, "ok");
    assert_eq!(f.app.project_path.as_ref(), Some(&second));
    assert_eq!(persistence::load(&second).unwrap(), f.app.project);
    assert_eq!(fs::read(&target).unwrap(), original);
}

#[test]
fn project_save_browser_cancellation_and_explicit_overwrite() {
    use voxmoviemaker::{project::persistence, ui::file_picker::FilePicker};
    let mut f = Fixture::new();
    f.prepare_transport();
    let target = f.root.join("existing.vmm.json");
    fs::write(&target, b"existing project").unwrap();
    let original = f.app.project.clone();
    f.app.file_picker = Some(FilePicker::built_in_save(
        PathMode::Save,
        f.app.project.project_uuid,
        target.clone(),
    ));
    f.app.save_current();
    let output = f.wait_for_browser_file("image.png");
    f.click_menu(&output, "cancel");
    assert!(f.app.file_picker.is_none());
    assert!(f.app.project_path.is_none());
    assert_eq!(f.app.project, original);
    assert!(f.app.dirty());
    assert_eq!(fs::read(&target).unwrap(), b"existing project");

    f.app.path_dialog.as_mut().unwrap().overwrite = true;
    f.app.file_picker = Some(FilePicker::built_in_save(
        PathMode::Save,
        f.app.project.project_uuid,
        target.clone(),
    ));
    let output = f.wait_for_browser_file("image.png");
    f.click_menu(&output, "select-export-path");
    assert!(
        !f.app.path_dialog.as_ref().unwrap().overwrite,
        "a changed destination needs new consent"
    );
    let output = f.frame(vec![]);
    f.click_menu(&output, "ok"); // disabled until replacement is explicitly allowed
    assert!(f.app.project_path.is_none());
    assert_eq!(fs::read(&target).unwrap(), b"existing project");
    let output = f.frame(vec![]);
    f.click_menu(&output, "confirm-overwrite");
    let output = f.frame(vec![]);
    f.click_menu(&output, "ok");
    assert!(f.app.path_dialog.is_none());
    assert_eq!(f.app.project_path.as_ref(), Some(&target));
    assert_eq!(persistence::load(&target).unwrap(), f.app.project);
}

fn deliver_import(
    f: &mut Fixture,
    track: usize,
    start: u64,
    results: Vec<voxmoviemaker::Result<MediaAsset>>,
) {
    let project = f.app.project.project_uuid;
    let target = f.app.project.tracks[track].id;
    f.app.importing = true;
    f.app
        .worker
        .submit(move || voxmoviemaker::app::Event::Imported(project, Some(target), start, results))
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while f.app.importing && Instant::now() < deadline {
        f.frame(vec![]);
        thread::sleep(Duration::from_millis(5));
    }
    assert!(!f.app.importing);
}
fn imported_asset(f: &Fixture, kind: MediaKind) -> MediaAsset {
    MediaAsset {
        id: Id::new_v4(),
        name: format!("new-{kind:?}"),
        location: AssetLocation::External(f.root.join("new-media")),
        kind,
        duration_seconds: 2.0,
        width: 32,
        height: 32,
        has_audio: kind != MediaKind::Image,
        sample_rate: Some(48000),
        frame_rate: None,
        codec: "test".into(),
        start_time: 0.0,
        rotation: 0,
        video_stream: (kind != MediaKind::Audio).then_some(0),
        audio_stream: (kind != MediaKind::Image).then_some(0),
    }
}
#[test]
fn overlapping_import_keeps_the_batch_together_and_preserves_existing_clips() {
    let mut f = Fixture::new();
    let background = f.image_clip(0);
    let blocked = f.image_clip(1);
    f.app.project.clip_mut(blocked).unwrap().timeline_start = 120;
    let before = f.app.project.clone();
    let video = imported_asset(&f, MediaKind::Video);
    let audio = imported_asset(&f, MediaKind::Audio);
    let video_id = video.id;
    let audio_id = audio.id;
    deliver_import(&mut f, 0, 30, vec![Ok(video), Ok(audio)]);
    assert_eq!(f.app.project.tracks[2].clips.len(), 2, "{}", f.app.status);
    assert_eq!(f.app.project.clip(background), before.clip(background));
    assert_eq!(f.app.project.clip(blocked), before.clip(blocked));
    let clips = &f.app.project.tracks[2].clips;
    assert_eq!((clips[0].timeline_start, clips[1].timeline_start), (30, 90));
    assert_eq!(clips[0].source.asset_id(), Some(video_id));
    assert_eq!(clips[1].source.asset_id(), Some(audio_id));
    assert_eq!(f.app.track, 2);
    assert_eq!(f.app.selected, Some(clips[0].id));
    f.app.project.validate().unwrap();
    f.app.history.undo(&mut f.app.project);
    assert_eq!(f.app.project, before);
    f.app.history.redo(&mut f.app.project);
    assert_eq!(f.app.project.tracks[2].clips.len(), 2);
}
#[test]
fn overlapping_voice_reuses_a_free_lower_layer_and_keeps_audio_and_subtitle_together() {
    let mut f = Fixture::new();
    let background = f.image_clip(0);
    let before = f.app.project.clone();
    let voice = imported_asset(&f, MediaKind::Audio);
    let voice_id = voice.id;
    let request = voxmoviemaker::app::VoiceRequest {
        character: None,
        project: f.app.project.project_uuid,
        track: f.app.project.tracks[0].id,
        start: 30,
        replace: None,
        original: None,
        text: "背景に重なる音声".into(),
        settings: Default::default(),
        audition: false,
    };
    f.app.generating = true;
    f.app
        .worker
        .submit(move || voxmoviemaker::app::Event::Voice(Box::new(request), Ok(voice)))
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while f.app.generating && Instant::now() < deadline {
        f.frame(vec![]);
        thread::sleep(Duration::from_millis(5));
    }
    assert!(!f.app.generating);
    assert_eq!(f.app.project.tracks.len(), before.tracks.len());
    assert_eq!(f.app.project.clip(background), before.clip(background));
    assert_eq!(f.app.project.tracks[1].clips.len(), 1, "{}", f.app.status);
    let clip = &f.app.project.tracks[1].clips[0];
    assert_eq!((clip.timeline_start, clip.duration), (30, 60));
    assert!(
        matches!(&clip.source, ClipSource::Voicevox { asset_id, subtitle: Some(style), .. } if *asset_id == voice_id && style.text == "背景に重なる音声")
    );
    assert_eq!(f.app.selected, Some(clip.id));
    f.app.history.undo(&mut f.app.project);
    assert_eq!(f.app.project, before);
}

#[test]
fn occupied_layers_create_and_reveal_one_new_layer_with_a_single_undo() {
    let mut f = Fixture::new();
    for index in 0..20 {
        if index == f.app.project.tracks.len() {
            f.app.project.tracks.push(Track::new(index + 1));
        }
        f.image_clip(index);
    }
    let before = f.app.project.clone();
    let asset = imported_asset(&f, MediaKind::Audio);
    deliver_import(&mut f, 0, 30, vec![Ok(asset)]);
    assert_eq!(f.app.project.tracks.len(), 21);
    let selected = f.app.selected.unwrap();
    assert_eq!(f.app.project.track_of(selected), Some(20));
    let mut visible = false;
    for _ in 0..40 {
        let output = f.frame(vec![]);
        visible = output
            .shapes
            .iter()
            .find_map(|s| text_position(&s.shape, "new-Audio"))
            .is_some_and(|point| f.app.timeline_rect.unwrap().contains(point));
        if visible {
            break;
        }
    }
    assert!(visible, "The inserted clip must scroll into view");
    f.app.history.undo(&mut f.app.project);
    assert_eq!(f.app.project, before);
}

#[test]
fn failed_blank_row_import_and_invalid_time_leave_no_empty_layers_or_assets() {
    let mut f = Fixture::new();
    let before = f.app.project.clone();
    f.app.import(
        vec![f.root.join("missing-file.mp4")],
        before.tracks.len(),
        0,
    );
    assert_eq!(f.app.project, before);
    let deadline = Instant::now() + Duration::from_secs(2);
    while f.app.importing && Instant::now() < deadline {
        f.frame(vec![]);
        thread::sleep(Duration::from_millis(5));
    }
    assert!(!f.app.importing);
    assert_eq!(f.app.project, before);
    let asset = imported_asset(&f, MediaKind::Video);
    deliver_import(&mut f, 0, MAX_FRAME - 10, vec![Ok(asset)]);
    assert_eq!(f.app.project, before);
    assert_eq!(f.app.status, f.app.tr("clip-outside-timeline"));
}

#[test]
fn voice_completion_never_redirects_to_a_different_layer_after_its_target_was_deleted() {
    let mut f = Fixture::new();
    let voice = imported_asset(&f, MediaKind::Audio);
    let request = voxmoviemaker::app::VoiceRequest {
        character: None,
        project: f.app.project.project_uuid,
        track: f.app.project.tracks[0].id,
        start: 0,
        replace: None,
        original: None,
        text: "deleted target".into(),
        settings: Default::default(),
        audition: false,
    };
    f.app.project.tracks.remove(0);
    let before = f.app.project.clone();
    f.app.generating = true;
    f.app
        .worker
        .submit(move || voxmoviemaker::app::Event::Voice(Box::new(request), Ok(voice)))
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while f.app.generating && Instant::now() < deadline {
        f.frame(vec![]);
        thread::sleep(Duration::from_millis(5));
    }
    assert!(!f.app.generating);
    assert_eq!(f.app.project, before);
    assert_eq!(f.app.status, f.app.tr("stale-result"));
}

#[test]
fn longer_voice_regeneration_keeps_original_when_it_would_overlap_a_neighbor() {
    let mut f = Fixture::new();
    let original_id = f.image_clip(0);
    let original_asset = f
        .app
        .project
        .clip(original_id)
        .unwrap()
        .source
        .asset_id()
        .unwrap();
    let asset = f
        .app
        .project
        .assets
        .iter_mut()
        .find(|a| a.id == original_asset)
        .unwrap();
    asset.kind = MediaKind::Audio;
    asset.has_audio = true;
    asset.duration_seconds = 5.0;
    let clip = f.app.project.clip_mut(original_id).unwrap();
    clip.duration = 30;
    clip.source = ClipSource::Voicevox {
        asset_id: original_asset,
        text: "original".into(),
        settings: Default::default(),
        subtitle: Some(TextSettings {
            text: "original".into(),
            ..Default::default()
        }),
    };
    let neighbor = f.image_clip(0);
    let clip = f.app.project.clip_mut(neighbor).unwrap();
    clip.timeline_start = 60;
    clip.duration = 30;
    let before = f.app.project.clone();
    let mut generated = imported_asset(&f, MediaKind::Audio);
    generated.duration_seconds = 3.0;
    let request = voxmoviemaker::app::VoiceRequest {
        character: None,
        project: before.project_uuid,
        track: before.tracks[0].id,
        start: 0,
        replace: Some(original_id),
        original: before.clip(original_id).cloned(),
        text: "longer new dialogue".into(),
        settings: Default::default(),
        audition: false,
    };
    f.app.generating = true;
    f.app
        .worker
        .submit(move || voxmoviemaker::app::Event::Voice(Box::new(request), Ok(generated)))
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while f.app.generating && Instant::now() < deadline {
        f.frame(vec![]);
        thread::sleep(Duration::from_millis(5));
    }
    assert!(!f.app.generating);
    assert_eq!(f.app.project, before);
    assert_eq!(f.app.status, f.app.tr("regeneration-collision"));
}

#[test]
fn natural_preview_completion_reaches_clip_end_and_keeps_the_last_picture() {
    let mut f = Fixture::new();
    let clip = f.prepare_transport();
    f.app.project.clip_mut(clip).unwrap().duration = 6;
    f.app.start_timeline(&f.ctx.clone());
    let deadline = Instant::now() + Duration::from_secs(3);
    while f.app.transport.active() && Instant::now() < deadline {
        f.frame(vec![]);
        thread::sleep(Duration::from_millis(5));
    }
    assert!(!f.app.transport.active());
    assert_eq!(
        f.app.playhead, 6,
        "The endpoint is 6, not the start time of the last picture"
    );
    assert!(
        f.app.texture.is_some(),
        "Finishing must retain the last decoded picture"
    );
}

#[test]
fn seeking_to_clip_end_stops_without_wrapping_and_play_restarts_from_zero() {
    let mut f = Fixture::new();
    f.prepare_transport();
    f.app.start_timeline(&f.ctx.clone());
    f.wait_for_session();
    let end = f.app.project.duration();
    f.app.seek_timeline(end, &f.ctx.clone());
    assert_eq!(f.app.playhead, end);
    assert!(!f.app.transport.active());
    f.app.start_timeline(&f.ctx.clone());
    f.wait_for_session();
    assert_eq!(f.app.transport.session.as_ref().unwrap().start, 0);
}

#[test]
fn fast_seek_drag_commits_the_release_position_at_zero() {
    for playing in [false, true] {
        let mut f = Fixture::new();
        f.prepare_transport();
        f.app.playhead = 90;
        if playing {
            f.app.start_timeline(&f.ctx.clone());
            f.wait_for_session();
        }
        f.frame(vec![]);
        let output = f.frame(vec![]);
        let label = output
            .shapes
            .iter()
            .find_map(|shape| text_position(&shape.shape, &f.app.tr("seek")))
            .unwrap();
        let rail = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::epaint::Shape::Rect(rect)
                    if (rect.rect.center().y - label.y).abs() < 2.0
                        && rect.rect.width() > 60.0
                        && rect.rect.height() < 12.0 =>
                {
                    Some(rect.rect)
                }
                _ => None,
            })
            .expect("seek slider rail");
        let grab = egui::pos2(rail.left() + rail.width() * 0.75, rail.center().y);
        f.frame(vec![Event::PointerMoved(grab)]);
        f.frame(vec![Fixture::button(grab, true)]);
        let middle = rail.center();
        f.frame(vec![Event::PointerMoved(middle)]);
        assert!(f.app.playhead > 0);
        // Linux can deliver the final motion and release in one update.
        let left = egui::pos2(rail.left() - 20.0, rail.center().y);
        f.frame(vec![
            Event::PointerMoved(left),
            Fixture::button(left, false),
        ]);
        assert_eq!(f.app.playhead, 0, "playing={playing}");
        if playing {
            f.wait_for_session();
            assert_eq!(f.app.transport.session.as_ref().unwrap().start, 0);
        } else {
            f.frame(vec![]);
            assert_eq!(f.app.playhead, 0);
        }
    }
}

#[test]
fn fast_timeline_seek_clamps_to_zero_beyond_the_ruler_start() {
    for playing in [false, true] {
        for release_with_motion in [false, true] {
            let mut f = Fixture::new();
            f.prepare_transport();
            f.frame(vec![]);
            f.frame(vec![]);
            if playing {
                f.app.start_timeline(&f.ctx.clone());
                f.wait_for_session();
            }
            let canvas = f.app.timeline_rect.unwrap();
            let grab = egui::pos2(
                canvas.left() + LABEL_WIDTH + 180.0,
                canvas.top() + RULER_HEIGHT / 2.0,
            );
            let middle = egui::pos2(canvas.left() + LABEL_WIDTH + 35.0, grab.y);
            let left = egui::pos2(canvas.left() + 10.0, grab.y);
            f.frame(vec![Event::PointerMoved(grab)]);
            f.frame(vec![Fixture::button(grab, true)]);
            f.frame(vec![Event::PointerMoved(middle)]);
            assert!(f.app.playhead > 0);
            if release_with_motion {
                f.frame(vec![
                    Event::PointerMoved(left),
                    Fixture::button(left, false),
                ]);
            } else {
                f.frame(vec![Event::PointerMoved(left)]);
                f.frame(vec![Fixture::button(left, false)]);
            }
            assert_eq!(
                f.app.playhead, 0,
                "playing={playing}, release_with_motion={release_with_motion}"
            );
            if playing {
                f.wait_for_session();
                assert_eq!(f.app.transport.session.as_ref().unwrap().start, 0);
            }
        }
    }
}

#[test]
fn dragging_from_timeline_labels_does_not_seek() {
    let mut f = Fixture::new();
    f.prepare_transport();
    f.app.playhead = 75;
    f.frame(vec![]);
    f.frame(vec![]);
    let canvas = f.app.timeline_rect.unwrap();
    let start = egui::pos2(canvas.left() + 10.0, canvas.top() + RULER_HEIGHT / 2.0);
    let end = egui::pos2(canvas.left() + LABEL_WIDTH + 100.0, start.y);
    f.frame(vec![Event::PointerMoved(start)]);
    f.frame(vec![Fixture::button(start, true)]);
    f.frame(vec![Event::PointerMoved(end)]);
    f.frame(vec![Fixture::button(end, false)]);
    assert_eq!(f.app.playhead, 75);
}

#[test]
fn first_picture_paint_time_is_excluded_from_the_preview_clock() {
    let mut f = Fixture::new();
    f.prepare_transport();
    f.app.start_timeline(&f.ctx.clone());
    let deadline = Instant::now() + Duration::from_secs(3);
    while f.app.texture.is_none() && Instant::now() < deadline {
        f.frame(vec![]);
        thread::sleep(Duration::from_millis(2));
    }
    assert!(f.app.texture.is_some());
    // Eframe paints after App::update returns. Model a slow first paint/present.
    thread::sleep(Duration::from_millis(200));
    assert_eq!(f.app.transport.session.as_ref().unwrap().frame(), 0);
    f.frame(vec![]); // the previous frame has now been painted
    thread::sleep(Duration::from_millis(40));
    f.frame(vec![]);
    assert!(f.app.playhead >= 1);
}

fn interactive_preview(f: &mut Fixture, selected: Id) -> egui::Pos2 {
    use voxmoviemaker::media::{
        preview::{self, EditPlanes, PreviewFrame},
        render_plan::RenderPlan,
    };
    f.app.tools = None;
    f.app.preview_dirty = false;
    let plan = RenderPlan::new(&f.app.project, &f.app.storage).unwrap();
    let objects = preview::objects(&plan, f.app.playhead, 640).unwrap();
    let object = objects.iter().find(|o| o.id == selected).unwrap();
    let clip = f.app.project.clip(selected).unwrap();
    let b = object.transformed(
        [f.app.project.width as f64, f.app.project.height as f64],
        &clip.effects,
        0,
    );
    let center = [(b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0];
    let rgba = |pixel| image::RgbaImage::from_pixel(160, 90, image::Rgba(pixel));
    f.app.install_preview(
        &f.ctx,
        PreviewFrame {
            image: rgba([0, 0, 0, 255]),
            objects,
            project: f.app.project.project_uuid,
            frame: f.app.playhead,
            planes: Some(EditPlanes {
                id: selected,
                behind: rgba([255, 0, 0, 255]),
                object: rgba([0, 255, 0, 255]),
                front: rgba([0; 4]),
            }),
        },
    );
    f.frame(vec![]);
    f.frame(vec![]);
    let rect = f.app.preview.rect.unwrap();
    rect.min
        + egui::vec2(
            center[0] as f32 / f.app.project.width as f32 * rect.width(),
            center[1] as f32 / f.app.project.height as f32 * rect.height(),
        )
}

#[test]
fn preview_drag_moves_all_visual_clip_types_with_one_undo_and_json_roundtrip() {
    for kind in ["image", "video", "text", "subtitle", "voice"] {
        let mut f = Fixture::new();
        let id = f.image_clip(0);
        let asset = f.app.project.assets.last_mut().unwrap();
        if kind == "video" {
            asset.kind = MediaKind::Video;
            asset.duration_seconds = 5.0;
        }
        if kind == "voice" {
            asset.kind = MediaKind::Audio;
            asset.has_audio = true;
            asset.duration_seconds = 5.0;
        }
        if matches!(kind, "text" | "subtitle" | "voice") {
            let settings = TextSettings {
                text: "Move me 日本語".into(),
                anchor: if kind == "text" {
                    TextAnchor::Center
                } else {
                    TextAnchor::Bottom
                },
                ..Default::default()
            };
            let source = if kind == "voice" {
                ClipSource::Voicevox {
                    asset_id: asset.id,
                    text: settings.text.clone(),
                    settings: Default::default(),
                    subtitle: Some(settings),
                }
            } else {
                ClipSource::Text { settings }
            };
            f.app.project.clip_mut(id).unwrap().source = source;
        }
        let before = f.app.project.clone();
        let start = interactive_preview(&mut f, id);
        let rect = f.app.preview.rect.unwrap();
        let end = start + egui::vec2(30.0, -15.0);
        f.frame(vec![Event::PointerMoved(start)]);
        f.frame(vec![Fixture::button(start, true)]);
        f.frame(vec![Event::PointerMoved(start + egui::vec2(12.0, -5.0))]);
        // Final motion can be delivered in the same update as release.
        let output = f.frame(vec![Event::PointerMoved(end), Fixture::button(end, false)]);
        assert_eq!(f.app.selected, Some(id), "{kind}");
        let effects = &f.app.project.clip(id).unwrap().effects;
        assert!(
            (effects.x - 30.0 * f.app.project.width as f64 / rect.width() as f64).abs() < 0.01,
            "{kind}: {effects:?}"
        );
        assert!(
            (effects.y + 15.0 * f.app.project.height as f64 / rect.height() as f64).abs() < 0.01
        );
        assert!(f.app.dirty());
        let current = f.app.preview.rect.unwrap();
        let expected_center = current.center()
            + egui::vec2(
                effects.x as f32 / f.app.project.width as f32 * current.width(),
                effects.y as f32 / f.app.project.height as f32 * current.height(),
            );
        assert!(output.shapes.iter().any(|s| matches!(&s.shape, egui::epaint::Shape::Mesh(mesh) if mesh.calc_bounds().center().distance(expected_center) < 1.0)), "{kind}: the object pixels must move immediately");
        let after = f.app.project.clone();
        let json = serde_json::to_string(&after).unwrap();
        assert_eq!(serde_json::from_str::<Project>(&json).unwrap(), after);
        f.app.history.undo(&mut f.app.project);
        assert_eq!(f.app.project, before, "one Undo must undo the whole drag");
        f.app.history.redo(&mut f.app.project);
        assert_eq!(f.app.project, after);
    }
}

fn wheel(point: egui::Pos2, delta: f32) -> Vec<Event> {
    vec![
        Event::PointerMoved(point),
        Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, delta),
            modifiers: Default::default(),
        },
    ]
}

#[test]
fn preview_wheel_scales_around_cursor_and_respects_limits_and_locks() {
    let mut f = Fixture::new();
    let id = f.image_clip(0);
    let start = interactive_preview(&mut f, id) + egui::vec2(15.0, 4.0);
    let rect = f.app.preview.rect.unwrap();
    f.frame(wheel(start, 80.0));
    let effects = &f.app.project.clip(id).unwrap().effects;
    assert!(effects.scale > 1.0);
    let x = (start.x - rect.center().x) as f64 * f.app.project.width as f64 / rect.width() as f64;
    assert!((x * effects.scale + effects.x - x).abs() < 0.01);
    f.frame(wheel(start, 1_000_000.0));
    assert_eq!(f.app.project.clip(id).unwrap().effects.scale, 4.0);
    f.frame(wheel(start, -1_000_000.0));
    assert_eq!(f.app.project.clip(id).unwrap().effects.scale, 0.05);
    f.app.project.tracks[0].locked = true;
    let point = interactive_preview(&mut f, id);
    let before = f.app.project.clone();
    f.frame(wheel(point, 80.0));
    f.frame(vec![
        Event::PointerMoved(point),
        Fixture::button(point, true),
    ]);
    f.frame(vec![Event::PointerMoved(point + egui::vec2(30.0, 0.0))]);
    f.frame(vec![Fixture::button(point, false)]);
    assert_eq!(f.app.project, before);
}

#[test]
fn preview_picks_foreground_and_escape_cancels_without_undo() {
    let mut f = Fixture::new();
    let back = f.image_clip(0);
    let front = f.image_clip(1);
    let start = interactive_preview(&mut f, front);
    let before = f.app.project.clone();
    f.frame(vec![
        Event::PointerMoved(start),
        Fixture::button(start, true),
    ]);
    f.frame(vec![Event::PointerMoved(start + egui::vec2(40.0, 0.0))]);
    assert_eq!(f.app.selected, Some(front));
    assert_eq!(f.app.project.clip(back).unwrap().effects.x, 0.0);
    f.frame(vec![Event::Key {
        key: egui::Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Default::default(),
    }]);
    f.frame(vec![Fixture::button(start, false)]);
    assert_eq!(f.app.project, before);
    f.app.history.undo(&mut f.app.project);
    assert_eq!(f.app.project, before);
}

#[test]
fn preview_drag_waits_for_async_frame_without_losing_the_release_position() {
    let mut f = Fixture::new();
    let id = f.image_clip(0);
    f.app.preview_dirty = false;
    f.app.preview.pending = true;
    f.frame(vec![]);
    f.frame(vec![]);
    let rect = f.app.preview.rect.unwrap();
    let start = rect.center();
    let end = start + egui::vec2(25.0, -10.0);
    f.frame(vec![
        Event::PointerMoved(start),
        Fixture::button(start, true),
    ]);
    f.frame(vec![Event::PointerMoved(end), Fixture::button(end, false)]);
    assert_eq!(f.app.project.clip(id).unwrap().effects.x, 0.0);
    // Moving the pointer elsewhere after release must not change the saved drag.
    f.frame(vec![Event::PointerMoved(end + egui::vec2(150.0, 0.0))]);
    interactive_preview(&mut f, id);
    assert_eq!(f.app.selected, Some(id));
    assert!(
        (f.app.project.clip(id).unwrap().effects.x
            - 25.0 * f.app.project.width as f64 / rect.width() as f64)
            .abs()
            < 0.01
    );
    f.app.history.undo(&mut f.app.project);
    assert_eq!(f.app.project.clip(id).unwrap().effects.x, 0.0);
}

#[test]
fn save_confirmation_resumes_close_and_open_but_not_after_failure() {
    let mut f = Fixture::new();
    f.prepare_transport();
    let path = f.root.join("pending.vmm.json");
    f.app.save_project(path.clone());
    f.app.project.tracks[0].name = "changed".into();
    f.app.confirm_close = true;
    f.frame(vec![]);
    let output = f.frame(vec![]);
    f.click_menu(&output, "save");
    assert!(!f.app.dirty());
    assert!(f.app.allow_close);
    assert!(!f.app.confirm_close);
    assert_eq!(
        voxmoviemaker::project::persistence::load(&path).unwrap(),
        f.app.project
    );

    f.app.allow_close = false;
    f.app.project.tracks[0].name = "open next".into();
    f.app.request_path(PathMode::Open);
    f.frame(vec![]);
    let output = f.frame(vec![]);
    f.click_menu(&output, "save");
    assert!(f.app.confirm_discard.is_none());
    assert_eq!(f.app.path_dialog.as_ref().unwrap().mode, PathMode::Open);
    f.app.path_dialog = None;

    let blocked = f.root.join("directory.vmm.json");
    fs::create_dir(&blocked).unwrap();
    f.app.project_path = Some(blocked);
    f.app.project.tracks[0].name = "keep this on error".into();
    let before = f.app.project.clone();
    f.app.confirm_close = true;
    f.frame(vec![]);
    let output = f.frame(vec![]);
    f.click_menu(&output, "save");
    assert!(f.app.confirm_close && !f.app.allow_close);
    assert!(f.app.dirty());
    assert_eq!(f.app.project, before);
}

#[test]
fn rejected_settings_draft_is_not_persisted_at_shutdown() {
    let mut f = Fixture::new();
    f.app.show_settings = true;
    let original = f.app.storage.data.clone();
    f.app.settings_data_path = Some(f.app.storage.cache.to_string_lossy().into_owned());
    let frame = |f: &mut Fixture, events: Vec<Event>| {
        f.ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1600.0, 2000.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| f.app.show(ctx),
        )
    };
    frame(&mut f, vec![]);
    let output = frame(&mut f, vec![]);
    let title = f.app.tr("settings");
    let pos = output
        .shapes
        .iter()
        .find_map(|s| text_position(&s.shape, &title))
        .unwrap();
    frame(
        &mut f,
        vec![Event::PointerMoved(pos + egui::vec2(200.0, 100.0))],
    );
    let mut output = frame(&mut f, vec![]);
    for _ in 0..8 {
        output = frame(
            &mut f,
            vec![Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, -500.0),
                modifiers: egui::Modifiers::NONE,
            }],
        );
    }
    let label = f.app.tr("apply-save");
    let pos = output
        .shapes
        .iter()
        .find_map(|s| text_position(&s.shape, &label))
        .unwrap();
    for event in [
        Event::PointerMoved(pos),
        Fixture::button(pos, true),
        Fixture::button(pos, false),
    ] {
        frame(&mut f, vec![event]);
    }
    assert_eq!(f.app.storage.data, original);
    assert!(f.app.settings.data_override.is_none());
    f.app.persist_settings();
    let (loaded, warning) = Settings::load(&f.app.storage);
    assert!(warning.is_none());
    assert!(loaded.data_override.is_none());
}

#[test]
fn property_drag_groups_updates_without_erasing_earlier_undo() {
    let mut f = Fixture::new();
    let id = f.image_clip(0);
    let before = f.app.project.clone();
    f.app.edit(|p| {
        p.tracks[1].name = "earlier operation".into();
        Ok(())
    });
    let baseline = f.app.project.clone();
    f.ctx.begin_pass(egui::RawInput {
        events: vec![Fixture::button(egui::pos2(300.0, 300.0), true)],
        ..Default::default()
    });
    let ctx = f.ctx.clone();
    for n in 1..=100 {
        f.app.edit_property(&ctx, id, |p| {
            p.clip_mut(id).unwrap().effects.x = n as f64;
            Ok(())
        });
    }
    let _ = f.ctx.end_pass();
    assert!(f.app.history.is_grouping());
    f.ctx.begin_pass(egui::RawInput {
        events: vec![Fixture::button(egui::pos2(300.0, 300.0), false)],
        ..Default::default()
    });
    f.app.finish_idle_property_edit(&ctx);
    let _ = f.ctx.end_pass();
    assert!(!f.app.history.is_grouping());
    assert_eq!(f.app.history.undo_len(), 2);
    let after = f.app.project.clone();
    f.app.history.undo(&mut f.app.project);
    assert_eq!(f.app.project, baseline);
    f.app.history.undo(&mut f.app.project);
    assert_eq!(f.app.project, before);
    f.app.history.redo(&mut f.app.project);
    f.app.history.redo(&mut f.app.project);
    assert_eq!(f.app.project, after);
}

#[test]
fn first_save_confirmation_can_choose_destination_and_then_close() {
    use voxmoviemaker::ui::file_picker::FilePicker;
    let mut f = Fixture::new();
    f.prepare_transport();
    f.app.confirm_close = true;
    let target = f.root.join("first-save.vmm.json");
    f.app.file_picker = Some(FilePicker::built_in_save(
        PathMode::Save,
        f.app.project.project_uuid,
        target.clone(),
    ));
    f.app.save_current();
    let output = f.wait_for_browser_file("image.png");
    f.click_menu(&output, "select-export-path");
    let output = f.frame(vec![]);
    f.click_menu(&output, "ok");
    assert!(target.is_file());
    assert!(f.app.allow_close && !f.app.confirm_close);
    assert!(!f.app.dirty());
}
