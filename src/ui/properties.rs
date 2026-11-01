use crate::{
    app::{App, PathMode},
    project::*,
};
use eframe::egui;

impl App {
    pub fn properties(&mut self, ui: &mut egui::Ui) {
        ui.heading(self.tr("properties"));
        if let Some(mut clip) = self.selected.and_then(|id| self.project.clip(id)).cloned() {
            let original = clip.clone();
            let track = self.project.track_of(clip.id).unwrap_or(0);
            let locked = self.project.tracks[track].locked;
            let asset = clip
                .source
                .asset_id()
                .and_then(|id| self.project.asset(id).ok())
                .cloned();
            if let Some(asset) = &asset {
                ui.label(&asset.name);
                let path = self.storage.resolve(self.project.project_uuid, asset);
                if !path.is_file() {
                    ui.colored_label(ui.visuals().error_fg_color, self.tr("missing-asset"));
                    if ui.button(self.tr("relink")).clicked() {
                        self.request_path(PathMode::Relink(asset.id));
                    }
                }
                ui.small(format!(
                    "{} · {}×{}",
                    asset.codec, asset.width, asset.height
                ));
            }
            ui.separator();
            ui.add_enabled_ui(!locked, |ui| {
                egui::Grid::new("clip-values")
                    .num_columns(2)
                    .spacing([8.0, 8.0])
                    .show(ui, |ui| {
                        ui.label(self.tr("start-frame"));
                        ui.add(egui::DragValue::new(&mut clip.timeline_start).range(0..=MAX_FRAME));
                        ui.end_row();
                        ui.label(self.tr("duration-frames"));
                        ui.add(egui::DragValue::new(&mut clip.duration).range(1..=MAX_FRAME));
                        ui.end_row();
                        if asset.as_ref().is_some_and(|a| a.kind != MediaKind::Image) {
                            ui.label(self.tr("trim-in"));
                            ui.add(
                                egui::DragValue::new(&mut clip.source_offset).range(0..=MAX_FRAME),
                            );
                            ui.end_row();
                        }
                        if asset.as_ref().is_some_and(|a| a.has_audio) {
                            ui.label(self.tr("volume"));
                            ui.add(egui::Slider::new(&mut clip.volume, 0.0..=4.0));
                            ui.end_row();
                            ui.label(self.tr("mute"));
                            ui.checkbox(&mut clip.muted, "");
                            ui.end_row();
                        }
                    });
                if let ClipSource::Text { settings } = &mut clip.source {
                    ui.separator();
                    ui.strong(self.tr(if settings.subtitle {
                        "subtitle"
                    } else {
                        "text-clip"
                    }));
                    self.text_properties(ui, settings);
                }
                if let ClipSource::Voicevox { text, subtitle, .. } = &mut clip.source {
                    ui.separator();
                    let mut enabled = subtitle.is_some();
                    if ui
                        .checkbox(&mut enabled, self.tr("voice-subtitle"))
                        .changed()
                    {
                        *subtitle = enabled.then(|| TextSettings {
                            text: text.clone(),
                            ..Default::default()
                        });
                    }
                    if let Some(style) = subtitle {
                        ui.small(self.tr("voice-subtitle-hint"));
                        self.text_properties(ui, style);
                    }
                }
                let visual = matches!(
                    clip.source,
                    ClipSource::Text { .. }
                        | ClipSource::Voicevox {
                            subtitle: Some(_),
                            ..
                        }
                ) || asset.as_ref().is_some_and(|a| a.kind != MediaKind::Audio);
                ui.separator();
                ui.strong(self.tr("clip-effects"));
                if visual {
                    ui.horizontal(|ui| {
                        ui.label("X");
                        ui.add(egui::DragValue::new(&mut clip.effects.x).range(-15360.0..=15360.0));
                        ui.label("Y");
                        ui.add(egui::DragValue::new(&mut clip.effects.y).range(-8640.0..=8640.0));
                    });
                    ui.add(
                        egui::Slider::new(&mut clip.effects.scale, 0.05..=4.0)
                            .text(self.tr("scale")),
                    );
                    ui.add(
                        egui::Slider::new(&mut clip.effects.opacity, 0.0..=1.0)
                            .text(self.tr("opacity")),
                    );
                    for (key, value) in [
                        ("fade-in", &mut clip.effects.fade_in),
                        ("fade-out", &mut clip.effects.fade_out),
                    ] {
                        ui.horizontal(|ui| {
                            ui.label(self.tr(key));
                            ui.add(egui::DragValue::new(value).range(0..=clip.duration));
                        });
                    }
                }
                if asset.as_ref().is_some_and(|a| a.has_audio) {
                    for (key, value) in [
                        ("audio-fade-in", &mut clip.effects.audio_fade_in),
                        ("audio-fade-out", &mut clip.effects.audio_fade_out),
                    ] {
                        ui.horizontal(|ui| {
                            ui.label(self.tr(key));
                            ui.add(egui::DragValue::new(value).range(0..=clip.duration));
                        });
                    }
                }
                ui.label(self.tr("transition"));
                egui::ComboBox::from_id_salt("clip-transition")
                    .selected_text(self.tr(match clip.effects.transition {
                        TransitionKind::None => "transition-none",
                        TransitionKind::Dissolve => "transition-dissolve",
                        TransitionKind::SlideLeft => "transition-slide",
                    }))
                    .show_ui(ui, |ui| {
                        for (kind, key) in [
                            (TransitionKind::None, "transition-none"),
                            (TransitionKind::Dissolve, "transition-dissolve"),
                            (TransitionKind::SlideLeft, "transition-slide"),
                        ] {
                            if !visual && kind == TransitionKind::SlideLeft {
                                continue;
                            }
                            ui.selectable_value(&mut clip.effects.transition, kind, self.tr(key));
                        }
                    });
                if clip.effects.transition != TransitionKind::None {
                    if clip.effects.transition_frames == 0 {
                        clip.effects.transition_frames =
                            self.project.frame_rate.frames(0.5).min(clip.duration);
                    }
                    ui.horizontal(|ui| {
                        ui.label(self.tr("transition-frames"));
                        ui.add(
                            egui::DragValue::new(&mut clip.effects.transition_frames)
                                .range(1..=clip.duration),
                        );
                    });
                }
                if ui.button(self.tr("reset-effects")).clicked() {
                    clip.effects = ClipEffects::default();
                }
            });
            if original != clip {
                clip.effects.clamp_duration(clip.duration);
                let max_duration = asset
                    .as_ref()
                    .filter(|a| a.kind != MediaKind::Image)
                    .map(|a| self.project.frame_rate.frames(a.duration_seconds));
                if max_duration
                    .is_some_and(|max| clip.source_offset.saturating_add(clip.duration) > max)
                {
                    self.status = self.tr("trim-outside-source");
                } else {
                    let id = clip.id;
                    self.edit(|p| {
                        *p.clip_mut(id).ok_or("missing-clip")? = clip;
                        Ok(())
                    });
                }
            }
            if let ClipSource::Voicevox { .. } = original.source {
                ui.separator();
                self.voice_properties(ui);
                ui.label(self.tr(if self.generating {
                    "generating"
                } else {
                    "voice-ready"
                }));
                if let Some(asset) = &asset
                    && ui.button(self.tr("play-generated")).clicked()
                {
                    self.play(self.storage.resolve(self.project.project_uuid, asset));
                }
            }
            ui.separator();
            if ui.button(self.tr("deselect")).clicked() {
                self.selected = None;
            }
        } else {
            ui.label(self.tr("project-settings"));
            let mut width = self.project.width;
            let mut height = self.project.height;
            ui.horizontal(|ui| {
                ui.label(self.tr("resolution"));
                ui.add(egui::DragValue::new(&mut width).range(2..=7680).speed(2.0));
                ui.label("×");
                ui.add(egui::DragValue::new(&mut height).range(2..=4320).speed(2.0));
            });
            if width != self.project.width || height != self.project.height {
                self.edit(|p| {
                    p.width = width / 2 * 2;
                    p.height = height / 2 * 2;
                    Ok(())
                });
            }
            let mut fps = self.project.frame_rate;
            egui::ComboBox::from_id_salt("fps")
                .selected_text(fps.ffmpeg())
                .show_ui(ui, |ui| {
                    for (n, d) in [
                        (24, 1),
                        (25, 1),
                        (30, 1),
                        (60, 1),
                        (24000, 1001),
                        (30000, 1001),
                    ] {
                        let rate = FrameRate {
                            numerator: n,
                            denominator: d,
                        };
                        ui.selectable_value(&mut fps, rate, rate.ffmpeg());
                    }
                });
            if fps != self.project.frame_rate {
                self.edit(|p| {
                    p.frame_rate = fps;
                    Ok(())
                });
            }
            ui.small(self.tr("fps-frames-warning"));
            ui.separator();
            self.voice_properties(ui);
        }
        ui.separator();
        ui.strong(self.tr("layer-settings"));
        if let Some(mut track) = self.project.tracks.get(self.track).cloned() {
            let original = track.clone();
            ui.label(format!("Layer {}", self.track + 1));
            ui.text_edit_singleline(&mut track.name);
            ui.checkbox(&mut track.enabled, self.tr("enabled"));
            ui.checkbox(&mut track.video_visible, self.tr("visible"));
            ui.checkbox(&mut track.muted, self.tr("mute"));
            ui.checkbox(&mut track.locked, self.tr("locked"));
            if original != track {
                let index = self.track;
                self.edit(|p| {
                    p.tracks[index] = track;
                    Ok(())
                });
            }
        }
    }
    fn text_properties(&self, ui: &mut egui::Ui, settings: &mut TextSettings) {
        ui.add(egui::TextEdit::multiline(&mut settings.text).desired_rows(3));
        ui.add(egui::Slider::new(&mut settings.size, 8.0..=256.0).text(self.tr("font-size")));
        ui.horizontal(|ui| {
            ui.label(self.tr("text-color"));
            ui.color_edit_button_srgba_unmultiplied(&mut settings.color);
        });
        ui.horizontal(|ui| {
            ui.label(self.tr("text-outline"));
            ui.color_edit_button_srgba_unmultiplied(&mut settings.outline);
        });
        ui.add(
            egui::Slider::new(&mut settings.outline_width, 0..=8).text(self.tr("outline-width")),
        );
        ui.horizontal_wrapped(|ui| {
            for (align, key) in [
                (TextAlign::Left, "align-left"),
                (TextAlign::Center, "align-center"),
                (TextAlign::Right, "align-right"),
            ] {
                ui.selectable_value(&mut settings.align, align, self.tr(key));
            }
        });
        ui.horizontal_wrapped(|ui| {
            for (anchor, key) in [
                (TextAnchor::Top, "anchor-top"),
                (TextAnchor::Center, "anchor-center"),
                (TextAnchor::Bottom, "anchor-bottom"),
            ] {
                ui.selectable_value(&mut settings.anchor, anchor, self.tr(key));
            }
        });
        ui.label(self.tr("font-path"));
        let mut font = settings.font.to_string_lossy().into_owned();
        if ui.text_edit_singleline(&mut font).changed() {
            settings.font = font.into();
        }
        ui.small(self.tr("font-auto"));
        ui.horizontal(|ui| {
            ui.label(self.tr("font-index"));
            ui.add(egui::DragValue::new(&mut settings.font_index).range(0..=64));
        });
    }
    fn voice_properties(&mut self, ui: &mut egui::Ui) {
        ui.strong("VOICEVOX");
        if ui
            .add_enabled(!self.connecting, egui::Button::new(self.tr("connect")))
            .clicked()
        {
            self.connect();
        }
        if let Some(connection) = &self.connection {
            ui.small(format!("VOICEVOX ENGINE {}", connection.version));
            egui::ComboBox::from_id_salt("speaker-style")
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
        } else {
            ui.small(self.tr("voicevox-offline"));
        }
        let speed = self.tr("speed");
        let pitch = self.tr("pitch");
        let intonation = self.tr("intonation");
        let volume = self.tr("voice-volume");
        let pre = self.tr("pre-phoneme");
        let post = self.tr("post-phoneme");
        ui.add(egui::Slider::new(&mut self.voice_settings.speed, 0.5..=2.0).text(speed));
        ui.add(egui::Slider::new(&mut self.voice_settings.pitch, -0.15..=0.15).text(pitch));
        ui.add(egui::Slider::new(&mut self.voice_settings.intonation, 0.0..=2.0).text(intonation));
        ui.add(egui::Slider::new(&mut self.voice_settings.volume, 0.0..=2.0).text(volume));
        ui.add(egui::Slider::new(&mut self.voice_settings.pre_phoneme, 0.0..=1.5).text(pre));
        ui.add(egui::Slider::new(&mut self.voice_settings.post_phoneme, 0.0..=1.5).text(post));
        ui.small(self.tr("voice-terms"));
    }
}
