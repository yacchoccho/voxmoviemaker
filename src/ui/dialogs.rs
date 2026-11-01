use crate::{
    app::{App, PathDialog, PathMode},
    assets::{ensure_outside_git, resolved_path},
    project::{commands::History, persistence},
    settings::ExportFormat,
};
use eframe::egui;
use std::{path::PathBuf, sync::atomic::Ordering};

impl App {
    pub fn dialogs(&mut self, ctx: &egui::Context) {
        self.settings_dialog(ctx);
        if let Some(mut picker) = self.file_picker.take() {
            match picker.show(ctx, &self.i18n) {
                None => self.file_picker = Some(picker),
                Some(Ok(None)) => {}
                Some(Err(error)) => {
                    self.report::<()>(Err(error));
                }
                Some(Ok(Some(path))) => {
                    if picker.project != self.project.project_uuid {
                        self.status = self.tr("stale-result");
                    } else {
                        match picker.mode {
                            PathMode::Import => {
                                if let Some(index) = self
                                    .project
                                    .tracks
                                    .iter()
                                    .position(|t| t.id == picker.track)
                                {
                                    self.import(vec![path], index, picker.start);
                                } else {
                                    self.status = self.tr("stale-result");
                                }
                            }
                            PathMode::Relink(id) => self.relink(id, path),
                            PathMode::Export | PathMode::Save => {
                                if let Some(dialog) =
                                    self.path_dialog.as_mut().filter(|d| d.mode == picker.mode)
                                {
                                    let path = if picker.mode == PathMode::Save {
                                        super::file_picker::project_filename(path)
                                    } else {
                                        super::file_picker::export_filename(
                                            path,
                                            self.settings.export_format,
                                        )
                                    };
                                    dialog.overwrite = false;
                                    dialog.path = path.to_string_lossy().into_owned();
                                    dialog.selected_path = Some(path);
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
        if self.show_about {
            let mut open = true;
            egui::Window::new(self.tr("about"))
                .open(&mut open)
                .show(ctx, |ui| {
                    ui.heading("VoxMovieMaker");
                    ui.label(self.tr("independent"));
                    ui.label("MIT · 0.1.0");
                    ui.label(self.tr("no-bundling"));
                    ui.label(self.tr("trademark-unchecked"));
                    ui.label(self.tr("voice-terms"));
                    ui.hyperlink_to("VOICEVOX", "https://voicevox.hiroshiba.jp/");
                    ui.label(self.tr("third-party-location"));
                    ui.label(self.tr("system-audio-notice"));
                    ui.label(self.tr("proxy-preview"));
                });
            self.show_about = open;
        }
        if let Some(mut dialog) = self.path_dialog.take() {
            let title = self.tr(match dialog.mode {
                PathMode::Open => "open",
                PathMode::Save => "save-as",
                PathMode::Import => "import",
                PathMode::Export => "export",
                PathMode::Package => "export-package",
                PathMode::Relink(_) => "relink",
            });
            let mut open = true;
            let mut execute = false;
            let mut choose_destination = false;
            egui::Window::new(title)
                .open(&mut open)
                .enabled(
                    !self
                        .file_picker
                        .as_ref()
                        .is_some_and(|p| p.mode == dialog.mode),
                )
                .collapsible(false)
                .default_width(520.0)
                .show(ctx, |ui| {
                    ui.label(self.tr(
                        if matches!(dialog.mode, PathMode::Save | PathMode::Export) {
                            "save-location"
                        } else {
                            "path"
                        },
                    ));
                    if dialog.mode == PathMode::Package {
                        ui.label(self.tr("package-description"));
                    }
                    if matches!(dialog.mode, PathMode::Save | PathMode::Export) {
                        ui.label(if dialog.selected_path.is_some() {
                            dialog.path.clone()
                        } else {
                            self.tr("no-export-path")
                        });
                        choose_destination = ui.button(self.tr("browse-export-path")).clicked();
                    } else {
                        ui.add(
                            egui::TextEdit::singleline(&mut dialog.path)
                                .desired_width(f32::INFINITY),
                        );
                    }
                    if dialog.mode == PathMode::Export {
                        let old_format = self.settings.export_format;
                        ui.horizontal(|ui| {
                            ui.selectable_value(
                                &mut self.settings.export_format,
                                ExportFormat::Mp4,
                                "MP4 · H.264 / AAC",
                            );
                            ui.selectable_value(
                                &mut self.settings.export_format,
                                ExportFormat::WebM,
                                "WebM · VP9 / Opus",
                            );
                        });
                        if old_format != self.settings.export_format
                            && let Some(path) = dialog.selected_path.take()
                        {
                            let path = super::file_picker::export_filename(
                                path,
                                self.settings.export_format,
                            );
                            dialog.path = path.to_string_lossy().into_owned();
                            dialog.selected_path = Some(path);
                        }
                        ui.small(self.tr("export-no-overwrite"));
                        if dialog
                            .selected_path
                            .as_ref()
                            .is_some_and(|p| std::fs::symlink_metadata(p).is_ok())
                        {
                            ui.colored_label(ui.visuals().error_fg_color, self.tr("output-exists"));
                        }
                    }
                    if dialog.mode == PathMode::Save {
                        ui.small(self.tr("project-extension"));
                        ui.small(self.tr("project-private"));
                        if dialog.selected_path.as_ref().is_some_and(|p| p.exists()) {
                            ui.checkbox(&mut dialog.overwrite, self.tr("confirm-overwrite"));
                        }
                    }
                    if dialog.mode == PathMode::Import {
                        ui.small(self.tr("drop-hint"));
                    }
                    execute = ui
                        .add_enabled(
                            if dialog.mode == PathMode::Export {
                                dialog
                                    .selected_path
                                    .as_ref()
                                    .is_some_and(|p| std::fs::symlink_metadata(p).is_err())
                            } else if dialog.mode == PathMode::Save {
                                dialog
                                    .selected_path
                                    .as_ref()
                                    .is_some_and(|p| !p.exists() || dialog.overwrite)
                            } else {
                                !dialog.path.trim().is_empty()
                            },
                            egui::Button::new(self.tr("ok")),
                        )
                        .clicked();
                });
            if choose_destination {
                let mode = dialog.mode;
                self.path_dialog = Some(dialog);
                self.choose_save_destination(mode);
                return;
            }
            if execute {
                let path = dialog
                    .selected_path
                    .clone()
                    .unwrap_or_else(|| PathBuf::from(&dialog.path));
                match dialog.mode {
                    PathMode::Open => {
                        if let Some(project) = self.report(persistence::load(&path)) {
                            self.cancel_preview_transform();
                            self.stop_audio();
                            self.file_picker = None;
                            self.project = project;
                            self.saved = self.project.clone();
                            self.project_path = Some(path);
                            self.history = History::default();
                            self.selected = None;
                            self.track = 0;
                            self.playhead = 0;
                            self.changed();
                        }
                    }
                    PathMode::Save => {
                        if path.exists() && !dialog.overwrite {
                            self.status = self.tr("confirm-overwrite");
                            self.path_dialog = Some(dialog);
                            return;
                        }
                        self.save_project(path);
                    }
                    PathMode::Import => self.import(vec![path], self.track, self.playhead),
                    PathMode::Export => self.start_export(path, ctx),
                    PathMode::Package => self.start_package(path, ctx),
                    PathMode::Relink(id) => self.relink(id, path),
                }
            } else if open {
                self.path_dialog = Some(dialog);
            } else if matches!(dialog.mode, PathMode::Save | PathMode::Export)
                && self
                    .file_picker
                    .as_ref()
                    .is_some_and(|p| p.mode == dialog.mode)
            {
                self.file_picker = None;
            }
        }
        if self.confirm_discard.is_some() || self.confirm_new || self.confirm_close {
            egui::Window::new(self.tr("discard-title"))
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.label(self.tr("discard-description"));
                    ui.horizontal(|ui| {
                        if ui.button(self.tr("save")).clicked() {
                            self.save_current();
                        }
                        if ui.button(self.tr("discard")).clicked() {
                            if self.confirm_close {
                                self.persist_settings();
                                self.allow_close = true;
                                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                            } else if self.confirm_new {
                                self.new_project();
                            } else if let Some(mode) = self.confirm_discard {
                                self.path_dialog = Some(PathDialog {
                                    mode,
                                    path: String::new(),
                                    overwrite: false,
                                    selected_path: None,
                                });
                            }
                            self.confirm_discard = None;
                            self.confirm_new = false;
                            self.confirm_close = false;
                        }
                        if ui.button(self.tr("cancel")).clicked() {
                            self.confirm_discard = None;
                            self.confirm_new = false;
                            self.confirm_close = false;
                        }
                    });
                });
        }
        if let Some(job) = &self.export {
            egui::Window::new(self.tr("exporting"))
                .collapsible(false)
                .show(ctx, |ui| {
                    ui.add(egui::ProgressBar::new(job.value).show_percentage());
                    if ui.button(self.tr("cancel")).clicked() {
                        job.cancel.store(true, Ordering::Relaxed);
                    }
                });
        }
        if let Some(job) = &self.package {
            egui::Window::new(self.tr("packaging"))
                .collapsible(false)
                .show(ctx, |ui| {
                    ui.add(egui::ProgressBar::new(job.value).show_percentage());
                    if ui.button(self.tr("cancel")).clicked() {
                        job.cancel.store(true, Ordering::Relaxed);
                    }
                });
        }
    }
    fn settings_dialog(&mut self, ctx: &egui::Context) {
        if !self.show_settings {
            return;
        }
        let mut open = true;
        let mut save = false;
        let old = self.settings.clone();
        egui::Window::new(self.tr("settings"))
            .open(&mut open)
            .default_width(540.0)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for (key, field) in [
                        ("ffmpeg-path", &mut self.settings.ffmpeg),
                        ("ffprobe-path", &mut self.settings.ffprobe),
                    ] {
                        ui.label(self.i18n.text(key));
                        let mut path = field.to_string_lossy().into_owned();
                        if ui.text_edit_singleline(&mut path).changed() {
                            *field = path.into();
                        }
                    }
                    if ui
                        .add_enabled(!self.detecting, egui::Button::new(self.tr("detect-tools")))
                        .clicked()
                    {
                        self.detect();
                    }
                    ui.small(self.tr("manual-ffmpeg"));
                    if let Some(tools) = &self.tools {
                        ui.label(&tools.ffmpeg);
                        ui.label(&tools.ffprobe);
                    }
                    ui.separator();
                    ui.label(self.tr("voicevox-url"));
                    ui.text_edit_singleline(&mut self.settings.voicevox_url);
                    let remote = self.tr("allow-remote");
                    ui.checkbox(&mut self.settings.allow_remote_voicevox, remote);
                    ui.small(self.tr("remote-description"));
                    if ui
                        .add_enabled(!self.connecting, egui::Button::new(self.tr("connect")))
                        .clicked()
                    {
                        self.connect();
                    }
                    ui.small(self.tr("manual-voicevox"));
                    ui.separator();
                    let quality = self.tr("preview-width");
                    ui.add(
                        egui::Slider::new(&mut self.settings.preview_width, 160..=1280)
                            .text(quality),
                    );
                    ui.label(self.tr("audio-device"));
                    ui.text_edit_singleline(&mut self.settings.audio_device);
                    let cache = self.tr("cache-limit");
                    ui.add(
                        egui::Slider::new(&mut self.settings.cache_limit_mb, 8..=4096).text(cache),
                    );
                    ui.label(self.tr("voice-storage"));
                    let mut path = self
                        .settings
                        .data_override
                        .as_ref()
                        .unwrap_or(&self.storage.data)
                        .to_string_lossy()
                        .into_owned();
                    if ui.text_edit_singleline(&mut path).changed() {
                        self.settings.data_override = Some(path.into());
                    }
                    ui.small(self.tr("storage-change-warning"));
                    ui.label(format!(
                        "{}: {}",
                        self.tr("cache"),
                        self.storage.cache.display()
                    ));
                    save = ui
                        .add_enabled(!self.generating, egui::Button::new(self.tr("apply-save")))
                        .clicked();
                });
            });
        self.show_settings = open;
        if old.ffmpeg != self.settings.ffmpeg || old.ffprobe != self.settings.ffprobe {
            self.pause_timeline();
            self.tools = None;
            self.preview_dirty = true;
        }
        if old.voicevox_url != self.settings.voicevox_url
            || old.allow_remote_voicevox != self.settings.allow_remote_voicevox
        {
            self.connection = None;
        }
        if old.preview_width != self.settings.preview_width
            || old.audio_device != self.settings.audio_device
            || old.cache_limit_mb != self.settings.cache_limit_mb
        {
            self.pause_timeline();
            self.preview_dirty = true;
        }
        if save {
            if let Some(path) = self.settings.data_override.clone() {
                let result = ensure_outside_git(&path).and_then(|()| {
                    let path = resolved_path(&path)?;
                    let cache = resolved_path(&self.storage.cache)?;
                    if path.starts_with(&cache) || cache.starts_with(&path) {
                        return Err("data-cache-overlap".into());
                    }
                    Ok(())
                });
                if self.report(result).is_none() {
                    return;
                }
                self.storage.data = path;
            }
            self.persist_settings();
            self.preview_dirty = true;
        }
    }
}
