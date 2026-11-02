use crate::{app::App, project::ClipSource};
use eframe::egui;

impl App {
    pub fn voice_input(&mut self, ctx: &egui::Context) {
        let input_id = egui::Id::new("voice-input-editor");
        let focused = ctx.memory(|m| m.has_focus(input_id));
        let ime_events = ctx.input(|i| {
            i.events
                .iter()
                .filter_map(|event| {
                    if let egui::Event::Ime(event) = event {
                        Some(event.clone())
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
        });
        // A conversion-confirming Enter must not submit, including when Commit
        // and Enter arrive together. Enabled alone is not active composition.
        let composing = self.voice_ime_composing;
        if focused {
            for event in &ime_events {
                match event {
                    egui::ImeEvent::Preedit(text) => self.voice_ime_composing = !text.is_empty(),
                    egui::ImeEvent::Commit(_) | egui::ImeEvent::Disabled => {
                        self.voice_ime_composing = false
                    }
                    egui::ImeEvent::Enabled => {}
                }
            }
        } else {
            self.voice_ime_composing = false;
        }
        let ime_confirm = ime_events.iter().any(|event| match event {
            egui::ImeEvent::Commit(_) => true,
            egui::ImeEvent::Preedit(text) => !text.is_empty(),
            _ => false,
        });
        let submit = focused && !composing && !self.voice_ime_composing && !ime_confirm
            && ctx.input(|i| i.events.iter().any(|event| matches!(event,
                egui::Event::Key { key: egui::Key::Enter, pressed: true, repeat: false, modifiers, .. } if modifiers.is_none())));
        if submit {
            ctx.input_mut(|i| {
                i.consume_key(egui::Modifiers::NONE, egui::Key::Enter);
            });
        }
        let style = ctx.style();
        let font = egui::TextStyle::Body.resolve(&style);
        let row = ctx.fonts_mut(|fonts| fonts.row_height(&font));
        let rows = self.voice_text.split('\n').count().clamp(1, 8);
        let toolbar_height = style
            .spacing
            .interact_size
            .y
            .max(row + style.spacing.button_padding.y * 2.0);
        let height = row * rows as f32
            + toolbar_height * 2.0
            + style.spacing.item_spacing.y * 2.0
            + 8.0
            + 12.0;
        let mut generate = false;
        let mut regenerate = false;
        let mut audition = false;
        let panel = egui::TopBottomPanel::bottom("voice")
            .resizable(false)
            .exact_height(height)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(self.tr("character"));
                    let mut selected = self.settings.character;
                    let mut state = std::mem::take(&mut self.character_ui.generation_search);
                    ui.allocate_ui_with_layout(
                        egui::vec2(250.0, ui.spacing().interact_size.y),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            self.character_picker(
                                ui,
                                &mut selected,
                                &mut state,
                                "generation-character",
                            );
                        },
                    );
                    self.settings.character = selected;
                    self.character_ui.generation_search = state;
                    if ui.button(self.tr("characters")).clicked() {
                        self.character_ui.open = true;
                        self.character_ui.selected = self.settings.character;
                        self.character_ui.draft = None;
                    }
                    if ui
                        .add_enabled(!self.connecting, egui::Button::new(self.tr("connect")))
                        .clicked()
                    {
                        self.connect();
                    }
                    if self.connecting {
                        ui.spinner();
                    }
                });
                ui.horizontal(|ui| {
                    ui.strong(self.tr("voice-input"));
                    if self.generating {
                        ui.spinner();
                        ui.label(self.tr("generating"));
                    }
                    let active = !self.generating && !self.voice_text.trim().is_empty();
                    let character_ready = self
                        .settings
                        .character
                        .is_some_and(|id| self.characters.library.get(id).is_some());
                    audition = ui
                        .add_enabled(
                            active && character_ready,
                            egui::Button::new(self.tr("audition")),
                        )
                        .clicked();
                    generate = ui
                        .add_enabled(
                            active && character_ready,
                            egui::Button::new(self.tr("generate-insert")),
                        )
                        .clicked();
                    let can_regenerate = self.selected.is_some_and(|id| {
                        self.project
                            .clip(id)
                            .is_some_and(|c| matches!(c.source, ClipSource::Voicevox { .. }))
                            && self
                                .project
                                .track_of(id)
                                .is_some_and(|index| !self.project.tracks[index].locked)
                    });
                    regenerate = ui
                        .add_enabled(
                            active && can_regenerate,
                            egui::Button::new(self.tr("regenerate")),
                        )
                        .clicked();
                    if self.playback.is_some() && ui.button(self.tr("stop")).clicked() {
                        self.stop_audio();
                    }
                });
                egui::ScrollArea::vertical()
                    .id_salt("voice-text")
                    .max_height(row * 8.0 + 8.0)
                    .show(ui, |ui| {
                        let hint = self.tr("voice-placeholder");
                        let response = ui
                            .add(
                                egui::TextEdit::multiline(&mut self.voice_text)
                                    .id(input_id)
                                    .desired_rows(1)
                                    .desired_width(f32::INFINITY)
                                    .return_key(egui::KeyboardShortcut::new(
                                        egui::Modifiers::SHIFT,
                                        egui::Key::Enter,
                                    ))
                                    .hint_text(hint),
                            )
                            .on_hover_text(self.tr("voice-input-keys"));
                        self.voice_input_rect = Some(response.rect);
                        if response.changed() {
                            ctx.request_repaint();
                        }
                    });
            });
        self.settings.voice_height = panel.response.rect.height();
        if audition {
            self.generate(true);
        }
        if generate || submit {
            self.generate(false);
        }
        if regenerate {
            self.regenerate_voice();
        }
    }
}
