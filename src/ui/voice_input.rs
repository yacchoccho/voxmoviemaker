use crate::{app::App, project::ClipSource};
use eframe::egui;

impl App {
    pub(super) fn voice_speaker_selector(&mut self, ui: &mut egui::Ui, location: &str) {
        let Some(connection) = &self.connection else {
            ui.small(self.tr("voicevox-offline"));
            return;
        };
        egui::ComboBox::from_id_salt(("speaker-style", location))
            .selected_text(format!(
                "{} / {}",
                self.voice_settings.speaker_name, self.voice_settings.style_name
            ))
            .show_ui(ui, |ui| {
                for speaker in &connection.speakers {
                    for style in &speaker.styles {
                        if ui
                            .selectable_label(
                                self.voice_settings.speaker == style.id,
                                format!("{} / {}", speaker.name, style.name),
                            )
                            .clicked()
                        {
                            self.voice_settings.speaker = style.id;
                            self.voice_settings.speaker_name = speaker.name.clone();
                            self.voice_settings.style_name = style.name.clone();
                        }
                    }
                }
            });
    }

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
        let height = row * rows as f32
            + style.spacing.interact_size.y * 2.0
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
                    ui.label(self.tr("speaker-style"));
                    self.voice_speaker_selector(ui, "voice-input");
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
                    audition = ui
                        .add_enabled(active, egui::Button::new(self.tr("audition")))
                        .clicked();
                    generate = ui
                        .add_enabled(active, egui::Button::new(self.tr("generate-insert")))
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
