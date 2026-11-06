use super::searchable_combo_box::{Choice, Labels, SearchState, searchable_combo_box};
use crate::{app::App, characters::*, project::*};
use eframe::egui;

#[derive(Default)]
pub struct CharacterUi {
    pub open: bool,
    pub selected: Option<Id>,
    pub draft: Option<Character>,
    pub tab: u8,
    pub delete: Option<Id>,
    pub generation_search: SearchState,
    pub clip_search: SearchState,
    manager_search: SearchState,
    font_search: SearchState,
    clip_font_search: SearchState,
    error: Option<String>,
    name_owner: Option<Id>,
    name_composing: bool,
}
impl App {
    pub fn save_characters(&mut self, library: CharacterLibrary) -> crate::Result<()> {
        self.characters.save(library)?;
        self.changed();
        Ok(())
    }
    pub fn reload_characters(&mut self) {
        let mut store = CharacterStore::load(&self.storage.config);
        if let Some(error) = &store.error {
            self.status = self.local_error(error);
            store.library = self.characters.library.clone();
        }
        self.characters = store;
        self.character_ui.draft = None;
        self.changed();
    }
    pub(super) fn character_picker(
        &self,
        ui: &mut egui::Ui,
        selected: &mut Option<Id>,
        state: &mut SearchState,
        salt: impl std::hash::Hash,
    ) -> bool {
        let mut counts = std::collections::HashMap::new();
        for character in &self.characters.library.characters {
            *counts.entry(&character.name).or_insert(0usize) += 1;
        }
        let choices: Vec<_> = self
            .characters
            .library
            .characters
            .iter()
            .map(|c| {
                let duplicate = counts.get(&c.name).is_some_and(|&count| count > 1);
                let name = if duplicate {
                    format!("{} [{}]", c.name, &c.id.to_string()[..8])
                } else {
                    c.name.clone()
                };
                Choice::new(c.id, name)
            })
            .collect();
        let fallback = if selected.is_some() {
            format!(
                "{} [{}]",
                self.tr("character-missing"),
                &selected.unwrap_or_default().to_string()[..8]
            )
        } else if choices.is_empty() {
            self.tr("character-create-first")
        } else {
            self.tr("character-select-first")
        };
        let mut id = selected.unwrap_or_default();
        let response = searchable_combo_box(
            ui,
            salt,
            state,
            &mut id,
            &choices,
            Labels {
                search: &self.tr("combo-search"),
                no_results: &self.tr("combo-no-results"),
                fallback: &fallback,
            },
        );
        if response.changed() {
            *selected = Some(id);
        }
        response.changed()
    }
    pub(super) fn character_window(&mut self, ctx: &egui::Context) {
        if !self.character_ui.open {
            return;
        }
        let mut state = std::mem::take(&mut self.character_ui);
        let mut open = state.open;
        egui::Window::new(self.tr("characters"))
            .id(egui::Id::new("character-manager"))
            .open(&mut open)
            .default_width(680.0)
            .default_height(560.0)
            .show(ctx, |ui| {
                if let Some(error) = &self.characters.error {
                    ui.colored_label(ui.visuals().error_fg_color, self.local_error(error));
                }
                if let Some(error) = &state.error {
                    ui.colored_label(ui.visuals().error_fg_color, self.local_error(error));
                }
                ui.horizontal(|ui| {
                    let previous = state.selected;
                    ui.allocate_ui_with_layout(
                        egui::vec2(240.0, ui.spacing().interact_size.y),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            self.character_picker(
                                ui,
                                &mut state.selected,
                                &mut state.manager_search,
                                "manager-character",
                            );
                        },
                    );
                    if previous != state.selected {
                        state.draft = None;
                    }
                    if ui.button(self.tr("character-new")).clicked() {
                        let character = Character::new(
                            self.tr("character-new-name"),
                            self.project.width,
                            self.project.height,
                        );
                        let id = character.id;
                        let mut library = self.characters.library.clone();
                        library.characters.push(character);
                        match self.save_characters(library) {
                            Ok(()) => {
                                state.selected = Some(id);
                                state.draft = None;
                                self.settings.character = Some(id);
                                state.error = None;
                            }
                            Err(error) => state.error = Some(error),
                        }
                    }
                    if ui
                        .add_enabled(
                            state.selected.is_some(),
                            egui::Button::new(self.tr("character-duplicate")),
                        )
                        .clicked()
                        && let Some(mut character) = state
                            .selected
                            .and_then(|id| self.characters.library.get(id))
                            .cloned()
                    {
                        character.id = Id::new_v4();
                        character.name =
                            format!("{} ({})", character.name, self.tr("character-copy"));
                        let id = character.id;
                        let mut library = self.characters.library.clone();
                        library.characters.push(character);
                        match self.save_characters(library) {
                            Ok(()) => {
                                state.selected = Some(id);
                                state.draft = None;
                                state.error = None;
                            }
                            Err(error) => state.error = Some(error),
                        }
                    }
                    if ui
                        .add_enabled(
                            state.selected.is_some(),
                            egui::Button::new(self.tr("delete")),
                        )
                        .clicked()
                    {
                        state.delete = state.selected;
                    }
                    if ui.button(self.tr("characters-reload")).clicked() {
                        self.reload_characters();
                        state.draft = None;
                        state.error = None;
                    }
                });
                if state.draft.is_none() {
                    state.draft = state
                        .selected
                        .and_then(|id| self.characters.library.get(id))
                        .cloned();
                }
                if let Some(draft) = &mut state.draft {
                    let before = draft.clone();
                    if state.name_owner != Some(draft.id) {
                        state.name_owner = Some(draft.id);
                        state.name_composing = false;
                    }
                    let mut name_confirmed = false;
                    let mut name_preedit = false;
                    ui.separator();
                    ui.horizontal(|ui| {
                        for (tab, key) in [
                            (0, "character-basic"),
                            (1, "character-voice"),
                            (2, "character-subtitle"),
                        ] {
                            ui.selectable_value(&mut state.tab, tab, self.tr(key));
                        }
                    });
                    egui::ScrollArea::vertical()
                        .id_salt("character-fields")
                        .max_height(480.0)
                        .show(ui, |ui| match state.tab {
                            0 => {
                                ui.label(self.tr("character-name"));
                                let response = ui.add(
                                    egui::TextEdit::singleline(&mut draft.name)
                                        // Error rows and changing picker labels must not change
                                        // the text editor's identity or its IME/cursor state.
                                        .id(egui::Id::new(("character-name", draft.id)))
                                        .return_key(None)
                                        .char_limit(256),
                                );
                                if response.has_focus() || response.lost_focus() {
                                    ui.input(|input| {
                                        for event in &input.events {
                                            if let egui::Event::Ime(event) = event {
                                                match event {
                                                    egui::ImeEvent::Preedit(text) => {
                                                        state.name_composing = !text.is_empty();
                                                        name_preedit = true;
                                                    }
                                                    egui::ImeEvent::Commit(_) => {
                                                        state.name_composing = false;
                                                        name_confirmed = true;
                                                    }
                                                    egui::ImeEvent::Disabled => {
                                                        state.name_composing = false
                                                    }
                                                    egui::ImeEvent::Enabled => {}
                                                }
                                            }
                                        }
                                    });
                                }
                                if !response.has_focus() {
                                    state.name_composing = false;
                                }
                                ui.label(format!("{}: VOICEVOX", self.tr("character-engine")));
                                ui.small(format!("ID: {}", draft.id));
                                self.voice_identity_editor(
                                    ui,
                                    &mut draft.voice,
                                    "character-identity",
                                );
                            }
                            1 => {
                                let mut overrides = VoiceOverrides::all(&draft.voice);
                                self.voice_override_editor(
                                    ui,
                                    &VoicevoxSettings::default(),
                                    &mut overrides,
                                    false,
                                );
                                draft.voice = overrides.apply(&VoicevoxSettings::default());
                            }
                            _ => {
                                let mut overrides = SubtitleOverrides::all(&draft.subtitle);
                                let base = TextSettings {
                                    position: Some([640.0, 660.0]),
                                    ..Default::default()
                                };
                                self.subtitle_override_editor(
                                    ui,
                                    &base,
                                    &mut overrides,
                                    false,
                                    draft.id,
                                    &mut state.font_search,
                                );
                                draft.subtitle = overrides.apply(&base);
                            }
                        });
                    if state.tab != 0 {
                        state.name_composing = false;
                    }
                    // Empty names are valid editing drafts, not valid persisted characters.
                    // Preedit must never reach the library. Commit can contain the same
                    // text as the preceding preedit, so it must also trigger a save.
                    let ready = !state.name_composing
                        && (!name_preedit || name_confirmed)
                        && !draft.name.trim().is_empty();
                    let save = ui
                        .add_enabled(ready, egui::Button::new(self.tr("characters-save")))
                        .clicked();
                    if ready && (*draft != before || name_confirmed || save) {
                        let mut library = self.characters.library.clone();
                        if let Some(value) =
                            library.characters.iter_mut().find(|c| c.id == draft.id)
                        {
                            *value = draft.clone();
                            match self.save_characters(library) {
                                Ok(()) => state.error = None,
                                Err(error) => state.error = Some(error),
                            }
                        }
                    }
                } else {
                    ui.label(self.tr("character-create-first"));
                }
            });
        if let Some(id) = state.delete {
            egui::Window::new(self.tr("character-delete-title"))
                .id(egui::Id::new("character-delete"))
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    if let Some(character) = self.characters.library.get(id) {
                        ui.label(format!("{} [{}]", character.name, character.id));
                    }
                    ui.label(self.tr("character-delete-warning"));
                    ui.horizontal(|ui| {
                        if ui.button(self.tr("delete")).clicked() {
                            let mut library = self.characters.library.clone();
                            library.characters.retain(|c| c.id != id);
                            match self.save_characters(library) {
                                Ok(()) => {
                                    state.delete = None;
                                    state.selected = None;
                                    state.draft = None;
                                    state.error = None;
                                }
                                Err(error) => state.error = Some(error),
                            }
                        }
                        if ui.button(self.tr("cancel")).clicked() {
                            state.delete = None;
                        }
                    });
                });
        }
        state.open = open;
        self.character_ui = state;
    }
    pub(super) fn clip_character_properties(&mut self, ui: &mut egui::Ui, clip: &mut Clip) {
        if matches!(
            clip.source,
            ClipSource::Media { .. } | ClipSource::Sprite { .. }
        ) {
            return;
        }
        ui.strong(self.tr("character"));
        let mut selected = clip.character.as_ref().map(|b| b.character_id);
        let mut search = std::mem::take(&mut self.character_ui.clip_search);
        if self.character_picker(ui, &mut selected, &mut search, ("clip-character", clip.id))
            && let Some(id) = selected
        {
            if let Some(binding) = &mut clip.character {
                binding.character_id = id;
            } else {
                clip.character = Some(Box::new(CharacterBinding::new(id)));
            }
        }
        self.character_ui.clip_search = search;
        if ui.button(self.tr("characters")).clicked() {
            self.character_ui.open = true;
            self.character_ui.selected = selected;
            self.character_ui.draft = None;
        }
        let Some(binding) = &mut clip.character else {
            ui.small(self.tr("character-legacy"));
            return;
        };
        if self.characters.library.get(binding.character_id).is_none() {
            ui.colored_label(ui.visuals().error_fg_color, self.tr("character-missing"));
        }
        if ui.button(self.tr("overrides-reset-all")).clicked() {
            binding.voice = Default::default();
            binding.subtitle = Default::default();
        }
        if matches!(clip.source, ClipSource::Voicevox { .. }) {
            ui.small(self.tr("voice-regenerate-required"));
            egui::CollapsingHeader::new(self.tr("character-voice"))
                .default_open(false)
                .show(ui, |ui| {
                    let base = self
                        .characters
                        .library
                        .get(binding.character_id)
                        .map(|c| c.voice.clone())
                        .unwrap_or_default();
                    let mut custom = binding.voice.identity.is_some();
                    if ui
                        .checkbox(&mut custom, self.tr("voice-identity-override"))
                        .changed()
                    {
                        binding.voice.identity =
                            custom.then(|| VoiceIdentity::from_settings(&base));
                    }
                    if let Some(identity) = &mut binding.voice.identity {
                        let mut settings = base.clone();
                        identity.apply(&mut settings);
                        self.voice_identity_editor(ui, &mut settings, "clip-voice-identity");
                        *identity = VoiceIdentity::from_settings(&settings);
                    }
                    self.voice_override_editor(ui, &base, &mut binding.voice, true);
                });
            ui.checkbox(&mut binding.subtitle_enabled, self.tr("voice-subtitle"));
        }
        if !matches!(clip.source, ClipSource::Voicevox { .. }) || binding.subtitle_enabled {
            egui::CollapsingHeader::new(self.tr("character-subtitle"))
                .default_open(true)
                .show(ui, |ui| {
                    let base = self
                        .characters
                        .library
                        .get(binding.character_id)
                        .map(|c| c.subtitle.clone())
                        .unwrap_or(TextSettings {
                            position: Some([640.0, 660.0]),
                            ..Default::default()
                        });
                    let mut search = std::mem::take(&mut self.character_ui.clip_font_search);
                    self.subtitle_override_editor(
                        ui,
                        &base,
                        &mut binding.subtitle,
                        true,
                        clip.id,
                        &mut search,
                    );
                    self.character_ui.clip_font_search = search;
                });
        }
    }
    pub(super) fn voice_identity_editor(
        &mut self,
        ui: &mut egui::Ui,
        settings: &mut VoicevoxSettings,
        salt: &str,
    ) {
        if ui
            .add_enabled(!self.connecting, egui::Button::new(self.tr("connect")))
            .clicked()
        {
            self.connect();
        }
        let Some(connection) = &self.connection else {
            ui.small(self.tr("voicevox-offline"));
            return;
        };
        ui.push_id(salt, |ui| {
            ui.label(self.tr("voice-speaker"));
            let old = settings.speaker_uuid.clone();
            egui::ComboBox::from_id_salt("speaker")
                .selected_text(if settings.speaker_name.is_empty() {
                    self.tr("voice-select")
                } else {
                    settings.speaker_name.clone()
                })
                .show_ui(ui, |ui| {
                    for speaker in &connection.speakers {
                        ui.selectable_value(
                            &mut settings.speaker_uuid,
                            speaker.speaker_uuid.clone(),
                            &speaker.name,
                        );
                    }
                });
            if let Some(speaker) = connection
                .speakers
                .iter()
                .find(|s| s.speaker_uuid == settings.speaker_uuid)
            {
                settings.speaker_name = speaker.name.clone();
                if old != settings.speaker_uuid
                    && let Some(style) = speaker.styles.first()
                {
                    settings.speaker = style.id;
                    settings.style_name = style.name.clone();
                }
                ui.label(self.tr("voice-style"));
                egui::ComboBox::from_id_salt("style")
                    .selected_text(&settings.style_name)
                    .show_ui(ui, |ui| {
                        for style in &speaker.styles {
                            if ui
                                .selectable_value(&mut settings.speaker, style.id, &style.name)
                                .clicked()
                            {
                                settings.style_name = style.name.clone();
                            }
                        }
                    });
                if !speaker.styles.iter().any(|s| s.id == settings.speaker) {
                    ui.small(self.tr("character-voice-missing"));
                }
            } else if !settings.speaker_uuid.is_empty() {
                ui.small(self.tr("character-voice-missing"));
            }
        });
    }
    pub(super) fn voice_override_editor(
        &self,
        ui: &mut egui::Ui,
        base: &VoicevoxSettings,
        values: &mut VoiceOverrides,
        inherited: bool,
    ) {
        for (key, value, default, range) in [
            ("speed", &mut values.speed, base.speed, 0.5..=2.0),
            ("pitch", &mut values.pitch, base.pitch, -0.15..=0.15),
            (
                "intonation",
                &mut values.intonation,
                base.intonation,
                0.0..=2.0,
            ),
            ("voice-volume", &mut values.volume, base.volume, 0.0..=2.0),
            (
                "pre-phoneme",
                &mut values.pre_phoneme,
                base.pre_phoneme,
                0.0..=1.5,
            ),
            (
                "post-phoneme",
                &mut values.post_phoneme,
                base.post_phoneme,
                0.0..=1.5,
            ),
        ] {
            self.override_number(ui, key, value, default, range, inherited);
        }
        for (key, value, default, range) in [
            (
                "pause-length",
                &mut values.pause_length,
                base.pause_length,
                0.0..=10.0,
            ),
            (
                "pause-scale",
                &mut values.pause_scale,
                base.pause_scale,
                0.0..=5.0,
            ),
        ] {
            ui.push_id(key, |ui| {
                ui.label(self.tr(key));
                let mut number = value.as_ref().map_or(default, |v| v.value);
                let mut enabled = number.is_some();
                if ui
                    .checkbox(&mut enabled, self.tr("voice-custom-value"))
                    .changed()
                {
                    number = enabled.then_some(default.unwrap_or(1.0));
                    *value = Some(Nullable { value: number });
                }
                if let Some(mut n) = number {
                    ui.horizontal(|ui| {
                        let slider =
                            ui.add(egui::Slider::new(&mut n, range.clone()).show_value(false));
                        let input = ui.add(egui::DragValue::new(&mut n).range(range).speed(0.01));
                        if slider.changed() || input.changed() {
                            *value = Some(Nullable { value: Some(n) });
                        }
                    });
                }
                self.inheritance_reset(ui, value, inherited);
            });
        }
        let mut rate = values.sampling_rate.unwrap_or(base.sampling_rate);
        ui.label(self.tr("voice-sampling-rate"));
        ui.horizontal(|ui| {
            let slider = ui.add(egui::Slider::new(&mut rate, 8000..=192000).show_value(false));
            let input = ui.add(
                egui::DragValue::new(&mut rate)
                    .range(8000..=192000)
                    .suffix(" Hz"),
            );
            if slider.changed() || input.changed() {
                values.sampling_rate = Some(rate);
            }
        });
        self.inheritance_reset(ui, &mut values.sampling_rate, inherited);
        for (key, value, default) in [
            ("voice-stereo", &mut values.stereo, base.stereo),
            (
                "voice-upspeak",
                &mut values.interrogative_upspeak,
                base.interrogative_upspeak,
            ),
        ] {
            ui.push_id(key, |ui| {
                let mut checked = value.unwrap_or(default);
                if ui.checkbox(&mut checked, self.tr(key)).changed() {
                    *value = Some(checked);
                }
                self.inheritance_reset(ui, value, inherited);
            });
        }
        ui.small(self.tr("voice-api-limits"));
    }
    fn override_number(
        &self,
        ui: &mut egui::Ui,
        key: &str,
        value: &mut Option<f64>,
        default: f64,
        range: std::ops::RangeInclusive<f64>,
        inherited: bool,
    ) {
        ui.push_id(key, |ui| {
            ui.label(self.tr(key));
            let mut number = value.unwrap_or(default);
            ui.horizontal(|ui| {
                let slider =
                    ui.add(egui::Slider::new(&mut number, range.clone()).show_value(false));
                let input = ui.add(egui::DragValue::new(&mut number).range(range).speed(0.01));
                if slider.changed() || input.changed() {
                    *value = Some(number);
                }
            });
            self.inheritance_reset(ui, value, inherited);
        });
    }
    fn inheritance_reset<T>(&self, ui: &mut egui::Ui, value: &mut Option<T>, inherited: bool) {
        ui.horizontal(|ui| {
            if inherited {
                ui.small(self.tr(if value.is_some() {
                    "value-overridden"
                } else {
                    "value-inherited"
                }));
            }
            if ui
                .add_enabled(
                    value.is_some(),
                    egui::Button::new(self.tr(if inherited {
                        "override-reset"
                    } else {
                        "value-default"
                    }))
                    .small(),
                )
                .clicked()
            {
                *value = None;
            }
        });
    }
    fn subtitle_override_editor(
        &mut self,
        ui: &mut egui::Ui,
        base: &TextSettings,
        values: &mut SubtitleOverrides,
        inherited: bool,
        target: Id,
        search: &mut SearchState,
    ) {
        let pos = base.position.unwrap_or([640.0, 660.0]);
        self.override_number(
            ui,
            "subtitle-x",
            &mut values.x,
            pos[0],
            -32768.0..=32768.0,
            inherited,
        );
        self.override_number(
            ui,
            "subtitle-y",
            &mut values.y,
            pos[1],
            -32768.0..=32768.0,
            inherited,
        );
        ui.small(self.tr("subtitle-pixels"));
        let mut style = values.apply(base);
        let before = style.clone();
        self.font_selector_with_state(ui, &mut style, target, search);
        if style.font != before.font || style.font_index != before.font_index {
            values.font = Some(style.font.clone());
            values.font_index = Some(style.font_index);
        }
        self.inheritance_reset(ui, &mut values.font, inherited);
        ui.label(self.tr("font-index"));
        let mut font_index = values.font_index.unwrap_or(base.font_index);
        if ui
            .add(egui::DragValue::new(&mut font_index).range(0..=64))
            .changed()
        {
            values.font_index = Some(font_index);
        }
        self.inheritance_reset(ui, &mut values.font_index, inherited);
        let mut size = values.size.map(f64::from);
        self.override_number(
            ui,
            "font-size",
            &mut size,
            base.size as f64,
            8.0..=256.0,
            inherited,
        );
        values.size = size.map(|v| v as f32);
        self.override_number(
            ui,
            "subtitle-opacity",
            &mut values.opacity,
            base.opacity,
            0.0..=1.0,
            inherited,
        );
        for (key, value, default) in [
            ("text-color", &mut values.color, base.color),
            ("text-outline", &mut values.outline, base.outline),
        ] {
            ui.push_id(key, |ui| {
                let mut color = value.unwrap_or(default);
                ui.horizontal(|ui| {
                    ui.label(self.tr(key));
                    if ui
                        .color_edit_button_srgba_unmultiplied(&mut color)
                        .changed()
                    {
                        *value = Some(color);
                    }
                });
                self.inheritance_reset(ui, value, inherited);
            });
        }
        let mut outline = values.outline_width.map(f64::from);
        self.override_number(
            ui,
            "outline-width",
            &mut outline,
            base.outline_width as f64,
            0.0..=8.0,
            inherited,
        );
        values.outline_width = outline.map(|v| v.round() as u32);
        ui.horizontal_wrapped(|ui| {
            let mut align = values.align.unwrap_or(base.align);
            for (value, key) in [
                (TextAlign::Left, "align-left"),
                (TextAlign::Center, "align-center"),
                (TextAlign::Right, "align-right"),
            ] {
                if ui
                    .selectable_value(&mut align, value, self.tr(key))
                    .changed()
                {
                    values.align = Some(align);
                }
            }
        });
        self.inheritance_reset(ui, &mut values.align, inherited);
        ui.horizontal_wrapped(|ui| {
            let mut anchor = values.anchor.unwrap_or(base.anchor);
            for (value, key) in [
                (TextAnchor::Top, "anchor-top"),
                (TextAnchor::Center, "anchor-center"),
                (TextAnchor::Bottom, "anchor-bottom"),
            ] {
                if ui
                    .selectable_value(&mut anchor, value, self.tr(key))
                    .changed()
                {
                    values.anchor = Some(anchor);
                }
            }
        });
        self.inheritance_reset(ui, &mut values.anchor, inherited);
    }
}
