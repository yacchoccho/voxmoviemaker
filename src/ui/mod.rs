mod dialogs;
pub mod file_picker;
pub mod preview;
mod properties;
pub mod timeline;
mod voice_input;
use crate::{
    app::{App, PathDialog, PathMode},
    project::{commands, persistence},
    settings::{ExportFormat, Theme},
};
use eframe::egui;
use std::sync::atomic::Ordering;

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.show(ctx);
    }
}
impl App {
    pub fn show(&mut self, ctx: &egui::Context) {
        if self.smoke_screenshot.is_some()
            && self.smoke_play_until.is_some()
            && !self.smoke_started
            && self.tools.is_some()
        {
            self.smoke_started = true;
            self.start_timeline(ctx);
        }
        if let Some(path) = self.smoke_screenshot.clone() {
            if ctx.cumulative_frame_nr() >= 5
                && !self.detecting
                && !self.preview_dirty
                && (self.tools.is_none() || self.texture.is_some())
                && !self.smoke_requested
                && self
                    .smoke_play_until
                    .is_none_or(|frame| self.transport.session.is_some() && self.playhead >= frame)
            {
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
                self.smoke_requested = true;
            }
            let image = ctx.input(|i| {
                i.events.iter().find_map(|e| {
                    if let egui::Event::Screenshot { image, .. } = e {
                        Some(image.clone())
                    } else {
                        None
                    }
                })
            });
            if let Some(image) = image {
                let bytes: Vec<u8> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
                if let Err(error) = image::save_buffer(
                    &path,
                    &bytes,
                    image.width() as u32,
                    image.height() as u32,
                    image::ColorType::Rgba8,
                ) {
                    eprintln!("Screenshot: {error}");
                }
                self.allow_close = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            ctx.request_repaint();
        }
        self.poll(ctx);
        if ctx.input(|i| i.viewport().close_requested()) && !self.allow_close {
            if self.dirty()
                || self.export.is_some()
                || self.package.is_some()
                || self.generating
                || self.importing
            {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.confirm_close = true;
            } else {
                self.persist_settings();
            }
        }
        if let Some(rect) = ctx.input(|i| i.viewport().inner_rect) {
            self.settings.window_size = [rect.width(), rect.height()];
        }
        self.shortcuts(ctx);
        egui::TopBottomPanel::top("menu").show(ctx, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.strong("VoxMovieMaker");
                if self.dirty() {
                    ui.label("*");
                }
                ui.menu_button(self.tr("file"), |ui| {
                    if ui.button(self.tr("new-project")).clicked() {
                        self.confirm_new = true;
                        ui.close();
                    }
                    if ui.button(self.tr("open")).clicked() {
                        self.request_path(PathMode::Open);
                        ui.close();
                    }
                    if ui.button(self.tr("save")).clicked() {
                        self.save_current();
                        ui.close();
                    }
                    if ui.button(self.tr("save-as")).clicked() {
                        self.request_path(PathMode::Save);
                        ui.close();
                    }
                    if ui.button(self.tr("import")).clicked() {
                        self.request_path(PathMode::Import);
                        ui.close();
                    }
                    if ui.button(self.tr("export")).clicked() {
                        self.request_path(PathMode::Export);
                        ui.close();
                    }
                    if ui.button(self.tr("export-package")).clicked() {
                        self.request_path(PathMode::Package);
                        ui.close();
                    }
                });
                ui.menu_button(self.tr("edit"), |ui| {
                    if ui.button(self.tr("undo")).clicked() {
                        self.finish_preview_transform();
                        self.history.undo(&mut self.project);
                        self.changed();
                        ui.close();
                    }
                    if ui.button(self.tr("redo")).clicked() {
                        self.finish_preview_transform();
                        self.history.redo(&mut self.project);
                        self.changed();
                        ui.close();
                    }
                    if ui.button(self.tr("copy")).clicked() {
                        self.clipboard =
                            self.selected.and_then(|id| self.project.clip(id)).cloned();
                        ui.close();
                    }
                    if ui.button(self.tr("paste")).clicked() {
                        self.paste();
                        ui.close();
                    }
                    if ui.button(self.tr("add-subtitle")).clicked() {
                        self.insert_text(true, false);
                        ui.close();
                    }
                    if ui.button(self.tr("add-text")).clicked() {
                        self.insert_text(false, false);
                        ui.close();
                    }
                    let voice = self
                        .selected
                        .and_then(|id| self.project.clip(id))
                        .is_some_and(|c| {
                            matches!(c.source, crate::project::ClipSource::Voicevox { .. })
                        });
                    if ui
                        .add_enabled(voice, egui::Button::new(self.tr("subtitle-from-voice")))
                        .clicked()
                    {
                        self.insert_text(true, true);
                        ui.close();
                    }
                    if ui.button(self.tr("split")).clicked() {
                        self.split_selected();
                        ui.close();
                    }
                    if ui.button(self.tr("delete")).clicked() {
                        self.delete_selected();
                        ui.close();
                    }
                });
                ui.menu_button(self.tr("view"), |ui| {
                    let mut changed = false;
                    for language in self.i18n.languages() {
                        changed |= ui
                            .selectable_value(
                                &mut self.settings.language,
                                language.clone(),
                                language,
                            )
                            .changed();
                    }
                    ui.separator();
                    for (theme, label) in [
                        (Theme::Dark, "dark"),
                        (Theme::Light, "light"),
                        (Theme::System, "system"),
                    ] {
                        let text = self.tr(label);
                        changed |= ui
                            .selectable_value(&mut self.settings.theme, theme, text)
                            .changed();
                    }
                    if changed {
                        self.i18n.language = self.settings.language.clone();
                        self.apply_appearance(ctx);
                        self.persist_settings();
                    }
                });
                if ui.button(self.tr("settings")).clicked() {
                    self.show_settings = true;
                }
                if ui.button(self.tr("help")).clicked() {
                    self.show_about = true;
                }
            });
        });
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(if self.tools.is_some() {
                    self.tr("ffmpeg-ready")
                } else {
                    self.tr("ffmpeg-missing")
                });
                ui.separator();
                ui.label(if self.connection.is_some() {
                    self.tr("voicevox-connected")
                } else {
                    self.tr("voicevox-offline")
                });
                ui.separator();
                ui.label(egui::RichText::new(&self.status).small())
                    .on_hover_text(&self.status);
            });
        });
        self.voice_input(ctx);
        let timeline_panel = egui::TopBottomPanel::bottom("timeline-panel")
            .resizable(true)
            .default_height(self.settings.timeline_height)
            .min_height(140.0)
            .show(ctx, |ui| self.timeline(ui, ctx));
        self.settings.timeline_height = timeline_panel.response.rect.height();
        let properties_panel = egui::SidePanel::right("properties")
            .resizable(true)
            .default_width(self.settings.properties_width)
            .min_width(220.0)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| self.properties(ui));
            });
        self.settings.properties_width = properties_panel.response.rect.width();
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.strong(self.tr("preview"));
                let active = self.transport.active();
                if ui
                    .add_enabled(
                        self.tools.is_some(),
                        egui::Button::new(self.tr(if active {
                            "pause-timeline"
                        } else {
                            "play-timeline"
                        })),
                    )
                    .clicked()
                {
                    self.toggle_timeline(ctx);
                }
                if ui
                    .add_enabled(
                        active || self.playhead > 0,
                        egui::Button::new(self.tr("stop-timeline")),
                    )
                    .clicked()
                {
                    self.stop_timeline();
                }
                if ui
                    .add_enabled(!active, egui::Button::new(self.tr("refresh")))
                    .clicked()
                {
                    self.preview_dirty = true;
                }
                let audio_label = self.tr("preview-audio");
                if ui
                    .checkbox(&mut self.settings.preview_audio, audio_label)
                    .changed()
                    && active
                {
                    self.seek_timeline(self.playhead, ctx);
                }
            });
            if self.transport.preparing.is_some() {
                ui.add(
                    egui::ProgressBar::new(
                        self.transport.progress.load(Ordering::Relaxed) as f32 / 1000.0,
                    )
                    .text(self.tr("proxy-preparing")),
                );
            } else {
                ui.small(format!(
                    "{} · {}: {}",
                    self.tr("proxy-preview"),
                    self.tr("skipped-frames"),
                    self.transport.skipped
                ));
            }
            ui.horizontal(|ui| {
                ui.label(self.tr("playhead"));
                let before = self.playhead;
                ui.add(
                    egui::DragValue::new(&mut self.playhead).range(0..=crate::project::MAX_FRAME),
                );
                ui.label(format!(
                    "{} s",
                    self.project.frame_rate.timestamp(self.playhead)
                ));
                if before != self.playhead {
                    self.seek_timeline(self.playhead, ctx);
                }
            });
            let duration = self.project.duration().max(1);
            let mut seek = self.playhead.min(duration);
            if ui
                .add(
                    egui::Slider::new(&mut seek, 0..=duration)
                        .show_value(false)
                        .text(self.tr("seek")),
                )
                .changed()
            {
                self.seek_timeline(seek, ctx);
            }
            let (area, _) = ui.allocate_exact_size(ui.available_size(), egui::Sense::hover());
            let source_size = self.texture.as_ref().map_or_else(
                || egui::vec2(self.project.width as f32, self.project.height as f32),
                |texture| texture.size_vec2(),
            );
            let rect =
                egui::Rect::from_center_size(area.center(), fit_aspect(area.size(), source_size));
            if let Some(texture) = &self.texture {
                egui::Image::new(texture).paint_at(ui, rect);
            } else {
                ui.painter().rect_filled(rect, 0.0, egui::Color32::BLACK);
                ui.painter().with_clip_rect(rect).text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    self.tr(if self.tools.is_some() {
                        "preview-pending"
                    } else {
                        "preview-no-tools"
                    }),
                    egui::FontId::proportional(15.0),
                    egui::Color32::LIGHT_GRAY,
                );
            }
            self.preview_interaction(ui, rect);
        });
        self.dialogs(ctx);
        if self.confirm_new && (!self.dirty() && !self.generating && !self.importing) {
            self.confirm_new = false;
            self.new_project();
        }
        self.present_timeline(ctx);
    }
}
fn fit_aspect(available: egui::Vec2, size: egui::Vec2) -> egui::Vec2 {
    size * (available.x / size.x).min(available.y / size.y).max(0.0)
}
impl App {
    pub fn request_path(&mut self, mode: PathMode) {
        if matches!(mode, PathMode::Import | PathMode::Relink(_)) {
            if self.file_picker.is_none() {
                let title = self.tr("choose-media");
                self.file_picker = Some(file_picker::FilePicker::new(
                    mode,
                    self.project.project_uuid,
                    self.project.tracks[self.track].id,
                    self.playhead,
                    &title,
                ));
            }
            return;
        }
        if mode == PathMode::Open && (self.dirty() || self.generating || self.importing) {
            self.confirm_discard = Some(mode);
            return;
        }
        let path = match mode {
            PathMode::Save => self
                .project_path
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|| "project.vmm.json".into()),
            PathMode::Export => String::new(),
            PathMode::Package => String::new(),
            _ => String::new(),
        };
        self.path_dialog = Some(PathDialog {
            mode,
            path,
            overwrite: false,
            selected_path: None,
        });
        if mode == PathMode::Save {
            self.choose_save_destination(mode);
        }
    }
    pub fn choose_save_destination(&mut self, mode: PathMode) {
        if self.file_picker.is_some() {
            return;
        }
        let name = if mode == PathMode::Save {
            "project.vmm.json"
        } else {
            match self.settings.export_format {
                ExportFormat::Mp4 => "output.mp4",
                ExportFormat::WebM => "output.webm",
            }
        };
        let initial = self
            .path_dialog
            .as_ref()
            .and_then(|dialog| dialog.selected_path.clone())
            .or_else(|| {
                self.project_path.as_ref().and_then(|p| {
                    if mode == PathMode::Save {
                        Some(p.clone())
                    } else {
                        p.parent().map(|p| p.join(name))
                    }
                })
            })
            .unwrap_or_else(|| std::path::PathBuf::from(name));
        let title = self.tr(if mode == PathMode::Save {
            "choose-project-path"
        } else {
            "choose-export-path"
        });
        self.file_picker = Some(file_picker::FilePicker::save(
            mode,
            self.project.project_uuid,
            &title,
            initial,
        ));
    }
    pub fn save_current(&mut self) {
        self.finish_preview_transform();
        if let Some(path) = self.project_path.clone() {
            self.save_project(path);
        } else {
            self.request_path(PathMode::Save);
        }
    }
    pub fn save_project(&mut self, path: std::path::PathBuf) {
        self.finish_preview_transform();
        if !path.to_string_lossy().ends_with(".vmm.json") {
            self.status = self.tr("project-extension");
            return;
        }
        let target = path.canonicalize().unwrap_or_else(|_| path.clone());
        if self.project.assets.iter().any(|a| {
            self.storage
                .resolve(self.project.project_uuid, a)
                .canonicalize()
                .ok()
                .as_ref()
                == Some(&target)
        }) {
            self.status = self.tr("source-protected");
            return;
        }
        if self
            .report(persistence::save(&path, &self.project))
            .is_some()
        {
            self.project_path = Some(path);
            self.saved = self.project.clone();
            self.status = self.tr("saved");
        }
    }
    pub fn split_selected(&mut self) {
        if let Some(id) = self.selected {
            let frame = self.playhead;
            self.edit(|p| commands::split(p, id, frame));
        }
    }
    pub fn delete_selected(&mut self) {
        if let Some(id) = self.selected {
            self.edit(|p| commands::delete(p, id));
        }
    }
    pub fn paste(&mut self) {
        if let Some(mut clip) = self.clipboard.clone() {
            if clip
                .source
                .asset_id()
                .is_some_and(|id| self.project.asset(id).is_err())
            {
                self.status = self.tr("missing-asset");
                return;
            }
            clip.id = crate::project::Id::new_v4();
            clip.timeline_start = self.playhead;
            let track = self.track;
            self.edit(|p| commands::insert(p, track, clip));
        }
    }
    fn shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.wants_keyboard_input() {
            return;
        }
        let key = |key, modifiers| ctx.input_mut(|i| i.consume_key(modifiers, key));
        if key(egui::Key::Z, egui::Modifiers::CTRL) {
            self.finish_preview_transform();
            self.history.undo(&mut self.project);
            self.changed();
        }
        if key(egui::Key::Y, egui::Modifiers::CTRL) {
            self.finish_preview_transform();
            self.history.redo(&mut self.project);
            self.changed();
        }
        if key(egui::Key::C, egui::Modifiers::CTRL) {
            self.clipboard = self.selected.and_then(|id| self.project.clip(id)).cloned();
        }
        if key(egui::Key::V, egui::Modifiers::CTRL) {
            self.paste();
        }
        if key(egui::Key::S, egui::Modifiers::CTRL) {
            self.save_current();
        }
        if key(egui::Key::O, egui::Modifiers::CTRL) {
            self.request_path(PathMode::Open);
        }
        if key(egui::Key::Delete, egui::Modifiers::NONE) {
            self.delete_selected();
        }
        if key(egui::Key::S, egui::Modifiers::NONE) {
            self.split_selected();
        }
        if ctx.input(|i| i.events.iter().any(|event| matches!(event, egui::Event::Key { key: egui::Key::Space, pressed: true, repeat: false, modifiers, .. } if modifiers.is_none()))) {
            self.toggle_timeline(ctx);
        }
        let mut frame = self.playhead;
        if key(egui::Key::ArrowLeft, egui::Modifiers::NONE) {
            frame = frame.saturating_sub(1);
        }
        if key(egui::Key::ArrowRight, egui::Modifiers::NONE) {
            frame = frame.saturating_add(1).min(crate::project::MAX_FRAME);
        }
        if frame != self.playhead {
            self.seek_timeline(frame, ctx);
        }
        if key(egui::Key::Escape, egui::Modifiers::NONE) {
            self.cancel_preview_transform();
            self.drag = None;
            self.stop_audio();
            self.pause_timeline();
            if let Some(job) = &self.export {
                job.cancel.store(true, Ordering::Relaxed);
            }
            if let Some(job) = &self.package {
                job.cancel.store(true, Ordering::Relaxed);
            }
        }
    }
}
