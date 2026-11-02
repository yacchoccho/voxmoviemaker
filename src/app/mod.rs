use crate::{
    Result,
    assets::Storage,
    i18n::I18n,
    media::{self, ffmpeg::Capabilities, render_plan::RenderPlan},
    project::{
        commands::{self, History},
        *,
    },
    settings::Settings,
    tasks::{PreviewRequest, PreviewWorker, Worker},
    voicevox::{self, Connection},
};
use eframe::egui;
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::Duration,
};
mod fonts;
mod import;
mod package;
mod property_edit;
mod text;
pub mod transport;

#[derive(Clone)]
pub struct VoiceRequest {
    pub project: Id,
    pub track: Id,
    pub start: u64,
    pub replace: Option<Id>,
    pub original: Option<Clip>,
    pub text: String,
    pub settings: VoicevoxSettings,
    pub audition: bool,
}
pub enum Event {
    Fonts(Result<Vec<media::fonts::SystemFont>>),
    Tools(PathBuf, PathBuf, Result<Capabilities>),
    Connection(String, Result<Connection>),
    Imported(Id, Option<Id>, u64, Vec<Result<MediaAsset>>),
    Voice(Box<VoiceRequest>, Result<MediaAsset>),
    Decoration(Id, PathBuf, Result<media::thumbnail::Decoration>),
    Relink(Id, Box<MediaAsset>, Result<MediaAsset>),
}
pub enum ClipDecoration {
    Image(egui::TextureHandle),
    Waveform(media::thumbnail::Waveform),
    Unavailable,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PathMode {
    Open,
    Save,
    Import,
    Export,
    Package,
    Relink(Id),
}
pub struct PathDialog {
    pub mode: PathMode,
    pub path: String,
    pub overwrite: bool,
    pub selected_path: Option<PathBuf>,
}
pub struct ExportJob {
    pub cancel: Arc<AtomicBool>,
    pub receiver: mpsc::Receiver<Result<()>>,
    pub progress: mpsc::Receiver<f32>,
    pub value: f32,
}
pub struct DragClip {
    pub id: Id,
    pub offset: f32,
    pub start: u64,
    pub track: usize,
}
pub struct App {
    pub project: Project,
    pub saved: Project,
    pub project_path: Option<PathBuf>,
    pub history: History,
    pub settings: Settings,
    pub storage: Storage,
    pub i18n: I18n,
    pub status: String,
    pub selected: Option<Id>,
    pub reveal_clip: Option<Id>,
    pub track: usize,
    pub playhead: u64,
    pub clipboard: Option<Clip>,
    pub voice_text: String,
    pub voice_input_rect: Option<egui::Rect>,
    pub voice_ime_composing: bool,
    pub voice_settings: VoicevoxSettings,
    pub fonts: fonts::FontCatalog,
    pub connection: Option<Connection>,
    pub tools: Option<Capabilities>,
    pub worker: Worker<Event>,
    pub preview_worker: PreviewWorker,
    pub preview_ticket: u64,
    pub preview_dirty: bool,
    pub texture: Option<egui::TextureHandle>,
    pub transport: transport::Transport,
    pub preview: crate::ui::preview::PreviewState,
    pub show_settings: bool,
    pub settings_data_path: Option<String>,
    pub property_edit: Option<property_edit::PropertyEdit>,
    pub resume_after_save: bool,
    pub show_about: bool,
    pub path_dialog: Option<PathDialog>,
    pub file_picker: Option<crate::ui::file_picker::FilePicker>,
    pub confirm_discard: Option<PathMode>,
    pub confirm_new: bool,
    pub confirm_close: bool,
    pub allow_close: bool,
    pub pixels_per_second: f32,
    pub snap: bool,
    pub drag: Option<DragClip>,
    pub timeline_rect: Option<egui::Rect>,
    pub drop_target: Option<(usize, u64)>,
    pub last_pointer: Option<egui::Pos2>,
    pub decorations: HashMap<Id, ClipDecoration>,
    pub decoration_pending: HashSet<Id>,
    pub export: Option<ExportJob>,
    pub package: Option<ExportJob>,
    pub generating: bool,
    pub connecting: bool,
    pub detecting: bool,
    pub importing: bool,
    pub playback: Option<Child>,
    pub revision: u64,
    pub smoke_screenshot: Option<PathBuf>,
    pub smoke_requested: bool,
    pub smoke_play_until: Option<u64>,
    pub smoke_started: bool,
}
impl App {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        storage: Storage,
        settings: Settings,
        warning: Option<String>,
    ) -> Self {
        let mut app = Self::from_context(cc.egui_ctx.clone(), storage, settings, warning);
        app.detect();
        app
    }
    pub fn from_context(
        ctx: egui::Context,
        storage: Storage,
        settings: Settings,
        warning: Option<String>,
    ) -> Self {
        let mut i18n = I18n::default();
        i18n.load_directory(&storage.config.join("locales"));
        i18n.language = settings.language.clone();
        let project = Project::default();
        let app = Self {
            saved: project.clone(),
            project,
            project_path: None,
            history: History::default(),
            settings,
            storage,
            i18n,
            status: warning.unwrap_or_default(),
            selected: None,
            reveal_clip: None,
            track: 0,
            playhead: 0,
            clipboard: None,
            voice_text: String::new(),
            voice_input_rect: None,
            voice_ime_composing: false,
            voice_settings: VoicevoxSettings::default(),
            fonts: Default::default(),
            connection: None,
            tools: None,
            worker: Worker::new(ctx.clone()),
            preview_worker: PreviewWorker::new(ctx.clone()),
            preview_ticket: 0,
            preview_dirty: true,
            texture: None,
            transport: transport::Transport::new(ctx.clone()),
            preview: Default::default(),
            show_settings: false,
            settings_data_path: None,
            property_edit: None,
            resume_after_save: false,
            show_about: false,
            path_dialog: None,
            file_picker: None,
            confirm_discard: None,
            confirm_new: false,
            confirm_close: false,
            allow_close: false,
            pixels_per_second: 70.0,
            snap: true,
            drag: None,
            timeline_rect: None,
            drop_target: None,
            last_pointer: None,
            decorations: HashMap::new(),
            decoration_pending: HashSet::new(),
            export: None,
            package: None,
            generating: false,
            connecting: false,
            detecting: false,
            importing: false,
            playback: None,
            revision: 0,
            smoke_screenshot: None,
            smoke_requested: false,
            smoke_play_until: None,
            smoke_started: false,
        };
        app.apply_appearance(&ctx);
        app
    }
    pub fn tr(&self, key: &str) -> String {
        self.i18n.text(key)
    }
    pub fn report<T>(&mut self, result: Result<T>) -> Option<T> {
        match result {
            Ok(value) => Some(value),
            Err(e) => {
                self.status = self.local_error(&e);
                None
            }
        }
    }
    pub fn local_error(&self, error: &str) -> String {
        let (key, detail) = error.split_once(": ").unwrap_or((error, ""));
        let translated = self.tr(key);
        if detail.is_empty() {
            translated
        } else {
            format!("{translated}: {detail}")
        }
    }
    pub fn dirty(&self) -> bool {
        self.project != self.saved
    }
    pub fn apply_appearance(&self, ctx: &egui::Context) {
        use crate::settings::Theme;
        ctx.set_theme(match self.settings.theme {
            Theme::Dark => egui::ThemePreference::Dark,
            Theme::Light => egui::ThemePreference::Light,
            Theme::System => egui::ThemePreference::System,
        });
    }
    pub fn edit(&mut self, edit: impl FnOnce(&mut Project) -> Result<()>) {
        self.finish_preview_transform();
        let result = self.history.edit(&mut self.project, edit);
        if self.report(result).is_some() {
            self.changed();
        }
    }
    pub fn changed(&mut self) {
        self.preview.scene = None;
        self.preview.pending = false;
        self.preview.retain = false;
        self.preview_worker.cancel();
        self.preview_ticket = 0;
        self.cancel_timeline();
        self.revision += 1;
        self.preview_dirty = true;
        if self
            .selected
            .is_some_and(|id| self.project.clip(id).is_none())
        {
            self.selected = None;
        }
        self.track = self.track.min(self.project.tracks.len().saturating_sub(1));
    }
    pub fn select(&mut self, id: Id) {
        if self.selected != Some(id) {
            self.finish_property_edit();
        }
        if self.selected == Some(id) {
            if self.voice_text.is_empty()
                && let Some(Clip {
                    source: ClipSource::Voicevox { text, settings, .. },
                    ..
                }) = self.project.clip(id)
            {
                self.voice_text = text.clone();
                self.voice_settings = settings.clone();
            }
            return;
        }
        self.selected = Some(id);
        self.preview_dirty = true;
        self.preview.retain = true;
        if let Some(index) = self.project.track_of(id) {
            self.track = index;
        }
        if let Some(Clip {
            source: ClipSource::Voicevox { text, settings, .. },
            ..
        }) = self.project.clip(id)
        {
            self.voice_text = text.clone();
            self.voice_settings = settings.clone();
        }
    }
    pub fn focus_clip(&mut self, id: Id) {
        self.select(id);
        self.reveal_clip = Some(id);
    }
    pub fn request_decoration(&mut self, id: Id) {
        if self.tools.is_none()
            || self.decorations.contains_key(&id)
            || self.decoration_pending.contains(&id)
            || self.decoration_pending.len() >= 2
        {
            return;
        }
        let Ok(asset) = self.project.asset(id) else {
            return;
        };
        let path = self.storage.resolve(self.project.project_uuid, asset);
        let ffmpeg = self.settings.ffmpeg.clone();
        let kind = asset.kind;
        let duration = asset.duration_seconds;
        if self
            .worker
            .submit(move || {
                let result = media::thumbnail::generate(&ffmpeg, &path, kind, duration);
                Event::Decoration(id, path, result)
            })
            .is_ok()
        {
            self.decoration_pending.insert(id);
        }
    }
    pub fn detect(&mut self) {
        if self.detecting {
            return;
        }
        let settings = self.settings.clone();
        let result = self.worker.submit(move || {
            Event::Tools(
                settings.ffmpeg.clone(),
                settings.ffprobe.clone(),
                media::ffmpeg::detect(&settings),
            )
        });
        self.detecting = self.report(result).is_some();
    }
    pub fn connect(&mut self) {
        if self.connecting {
            return;
        }
        let settings = self.settings.clone();
        let result = self.worker.submit(move || {
            let result =
                voicevox::Client::new(&settings.voicevox_url, settings.allow_remote_voicevox)
                    .and_then(|client| client.connect());
            Event::Connection(settings.voicevox_url, result)
        });
        self.connecting = self.report(result).is_some();
    }
    pub fn import(&mut self, paths: Vec<PathBuf>, track: usize, start: u64) {
        if self.importing {
            self.status = self.tr("worker-busy");
            return;
        }
        if paths.len() > 32 || paths.is_empty() {
            self.status = self.tr("too-many-files");
            return;
        }
        if track > self.project.tracks.len() {
            self.status = self.tr("invalid-track");
            return;
        }
        // None means the blank drop row. Create it only after successful probing.
        let target = self.project.tracks.get(track).map(|t| t.id);
        let project = self.project.project_uuid;
        let ffprobe = self.settings.ffprobe.clone();
        let result = self.worker.submit(move || {
            Event::Imported(
                project,
                target,
                start,
                paths
                    .into_iter()
                    .map(|p| media::ffprobe::probe(&ffprobe, &p))
                    .collect(),
            )
        });
        self.importing = self.report(result).is_some();
    }
    pub fn relink(&mut self, id: Id, path: PathBuf) {
        if self.importing {
            self.status = self.tr("worker-busy");
            return;
        }
        let Some(original) = self.project.asset(id).ok().cloned() else {
            return;
        };
        let project = self.project.project_uuid;
        let ffprobe = self.settings.ffprobe.clone();
        let result = self.worker.submit(move || {
            Event::Relink(
                project,
                Box::new(original),
                media::ffprobe::probe(&ffprobe, &path),
            )
        });
        self.importing = self.report(result).is_some();
    }
    pub fn generate(&mut self, audition: bool) {
        self.generate_voice(audition, None);
    }
    pub fn regenerate_voice(&mut self) {
        let Some(id) = self.selected.filter(|id| {
            self.project
                .clip(*id)
                .is_some_and(|c| matches!(c.source, ClipSource::Voicevox { .. }))
        }) else {
            return;
        };
        self.generate_voice(false, Some(id));
    }
    fn generate_voice(&mut self, audition: bool, replace: Option<Id>) {
        if self.generating || self.voice_text.trim().is_empty() {
            return;
        }
        if !audition
            && (TextSettings {
                text: self.voice_text.clone(),
                ..Default::default()
            })
            .validate()
            .is_err()
        {
            self.status = self.tr("voice-subtitle-too-long");
            return;
        }
        let Some(track) = self.project.tracks.get(self.track).map(|t| t.id) else {
            return;
        };
        let original = replace.and_then(|id| self.project.clip(id)).cloned();
        if let Some(id) = replace
            && self
                .project
                .track_of(id)
                .is_some_and(|i| self.project.tracks[i].locked)
        {
            self.status = self.tr("track-locked");
            return;
        }
        let request = VoiceRequest {
            project: self.project.project_uuid,
            track,
            start: self.playhead,
            replace,
            original,
            text: self.voice_text.clone(),
            settings: self.voice_settings.clone(),
            audition,
        };
        let storage = self.storage.clone();
        let settings = self.settings.clone();
        let result = self.worker.submit(move || {
            let result =
                voicevox::Client::new(&settings.voicevox_url, settings.allow_remote_voicevox)
                    .and_then(|client| client.synthesize(&request.text, &request.settings))
                    .and_then(|(bytes, _, _)| {
                        voicevox::persist(&storage, request.project, &bytes, &request.settings)
                    });
            Event::Voice(Box::new(request), result)
        });
        self.generating = self.report(result).is_some();
        if self.generating && !audition {
            self.voice_text.clear();
        }
    }
    fn receive_voice(&mut self, request: VoiceRequest, result: Result<MediaAsset>) {
        let draft = self.voice_text.clone();
        let settings = self.voice_settings.clone();
        let retry = request.text.clone();
        let restore = request.project == self.project.project_uuid && !request.audition;
        let success = self.place_voice(request, result);
        // Automatic selection must not reload the generated phrase over the next draft.
        if restore {
            self.voice_text = if !success && draft.is_empty() {
                retry
            } else {
                draft
            };
            self.voice_settings = settings;
        }
    }
    fn place_voice(&mut self, request: VoiceRequest, result: Result<MediaAsset>) -> bool {
        self.generating = false;
        let Some(asset) = self.report(result) else {
            return false;
        };
        if request.project != self.project.project_uuid {
            self.status = self.tr("stale-result");
            return false;
        }
        if request.audition {
            self.play(self.storage.resolve(request.project, &asset));
            return true;
        }
        if let Some(id) = request.replace {
            if self.project.clip(id) != request.original.as_ref() {
                self.status = self.tr("stale-result");
                return false;
            }
            let Some(track) = self.project.track_of(id) else {
                return false;
            };
            if self.project.tracks[track].locked {
                self.status = self.tr("track-locked");
                return false;
            }
        }
        let duration = self.project.frame_rate.frames(asset.duration_seconds);
        let target = if let Some(id) = request.replace {
            self.project.track_of(id)
        } else {
            self.project
                .tracks
                .iter()
                .position(|t| t.id == request.track)
        };
        let Some(target) = target else {
            self.status = self.tr("stale-result");
            return false;
        };
        let start = request
            .original
            .as_ref()
            .filter(|_| request.replace.is_some())
            .map_or(request.start, |c| c.timeline_start);
        let clip_id = request.replace.unwrap_or_else(Id::new_v4);
        let original = request
            .original
            .as_ref()
            .filter(|_| request.replace.is_some());
        let mut effects = original.map(|c| c.effects.clone()).unwrap_or_default();
        effects.clamp_duration(duration);
        let subtitle = match original.map(|c| &c.source) {
            Some(ClipSource::Voicevox { subtitle, .. }) => subtitle.clone(),
            _ => Some(TextSettings::default()),
        }
        .map(|mut style| {
            style.text = request.text.clone();
            style.subtitle = true;
            style
        });
        let clip = Clip {
            id: clip_id,
            source: ClipSource::Voicevox {
                asset_id: asset.id,
                text: request.text,
                settings: request.settings,
                subtitle,
            },
            timeline_start: start,
            duration,
            source_offset: 0,
            volume: original.map_or(1.0, |c| c.volume),
            muted: original.is_some_and(|c| c.muted),
            effects,
        };
        // Regeneration is transactional. A longer result must never remove or move the old clip on failure.
        if request.replace.is_some()
            && !self.project.tracks[target].fits(start, duration, request.replace)
        {
            self.status = self.tr("regeneration-collision");
            return false;
        }
        let replace = request.replace;
        self.finish_preview_transform();
        let result = self.history.edit(&mut self.project, move |p| {
            let destination = if replace.is_some() {
                target
            } else {
                commands::placement_track(p, target, start, duration)?
            };
            if let Some(id) = replace {
                commands::delete(p, id)?;
            }
            p.assets.push(asset);
            commands::insert(p, destination, clip)?;
            Ok(destination)
        });
        let Some(destination) = self.report(result) else {
            return false;
        };
        self.changed();
        self.selected = None;
        self.focus_clip(clip_id);
        self.status = self.tr(if destination != target {
            "inserted-safe-layer"
        } else {
            "voice-ready"
        });
        true
    }
    pub fn play(&mut self, path: PathBuf) {
        self.pause_timeline();
        self.stop_audio();
        for binary in ["pw-play", "aplay"] {
            let mut command = Command::new(binary);
            command
                .arg(&path)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            media::ffmpeg::protect_child(&mut command);
            if let Ok(child) = command.spawn() {
                self.playback = Some(child);
                return;
            }
        }
        self.status = self.tr("audio-player-missing");
    }
    pub fn stop_audio(&mut self) {
        if let Some(mut child) = self.playback.take() {
            media::ffmpeg::terminate(&mut child);
        }
    }
    pub fn start_export(&mut self, target: PathBuf, ctx: &egui::Context) {
        self.finish_preview_transform();
        if self.export.is_some() {
            return;
        }
        let Some(capabilities) = self.tools.clone() else {
            self.status = self.tr("ffmpeg-missing");
            return;
        };
        let Some(plan) = self.report(RenderPlan::new(&self.project, &self.storage)) else {
            return;
        };
        let settings = self.settings.clone();
        let cancel = media::ffmpeg::cancellation();
        let flag = cancel.clone();
        let (tx, receiver) = mpsc::channel();
        let (ptx, progress) = mpsc::sync_channel(8);
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = media::export::export(
                &plan,
                &settings.ffmpeg,
                &capabilities,
                settings.export_format,
                &target,
                flag,
                |value| {
                    let _ = ptx.try_send(value);
                    ctx.request_repaint();
                },
            );
            let _ = tx.send(result);
            ctx.request_repaint();
        });
        self.export = Some(ExportJob {
            cancel,
            receiver,
            progress,
            value: 0.0,
        });
    }
    pub fn poll(&mut self, ctx: &egui::Context) {
        while let Ok(event) = self.worker.receiver.try_recv() {
            match event {
                Event::Fonts(result) => {
                    self.fonts.loading = false;
                    match result {
                        Ok(fonts) => {
                            self.fonts.choices =
                                std::iter::once(crate::ui::searchable_combo_box::Choice::new(
                                    media::fonts::FontId::default(),
                                    self.tr("font-automatic"),
                                ))
                                .chain(fonts.into_iter().map(|font| {
                                    crate::ui::searchable_combo_box::Choice::new(font.id, font.name)
                                }))
                                .collect();
                            self.fonts.loaded = true;
                            self.fonts.error = None;
                        }
                        Err(error) => self.fonts.error = Some(error),
                    }
                }
                Event::Tools(ffmpeg, ffprobe, result) => {
                    self.detecting = false;
                    if ffmpeg == self.settings.ffmpeg && ffprobe == self.settings.ffprobe {
                        self.tools =
                            self.report(result.map_err(|e| format!("ffmpeg-missing: {e}")));
                        self.preview_dirty = true;
                    }
                }
                Event::Connection(url, result) => {
                    self.connecting = false;
                    if url == self.settings.voicevox_url {
                        self.connection =
                            self.report(result.map_err(|e| format!("voicevox-unavailable: {e}")));
                        if let Some(speaker) =
                            self.connection.as_ref().and_then(|c| c.speakers.first())
                            && let Some(style) = speaker.styles.first()
                            && self.voice_settings.speaker_name.is_empty()
                        {
                            self.voice_settings.speaker = style.id;
                            self.voice_settings.speaker_name = speaker.name.clone();
                            self.voice_settings.style_name = style.name.clone();
                        }
                    }
                }
                Event::Imported(project, track, start, results) => {
                    self.receive_import(project, track, start, results);
                }
                Event::Voice(request, result) => self.receive_voice(*request, result),
                Event::Decoration(id, path, result) => {
                    self.decoration_pending.remove(&id);
                    if self
                        .project
                        .asset(id)
                        .is_ok_and(|a| self.storage.resolve(self.project.project_uuid, a) == path)
                    {
                        let decoration = match result {
                            Ok(media::thumbnail::Decoration::Image(image)) => {
                                ClipDecoration::Image(ctx.load_texture(
                                    format!("asset-{id}"),
                                    egui::ColorImage::from_rgba_unmultiplied(
                                        [image.width() as usize, image.height() as usize],
                                        image.as_raw(),
                                    ),
                                    egui::TextureOptions::LINEAR,
                                ))
                            }
                            Ok(media::thumbnail::Decoration::Waveform(bins)) => {
                                ClipDecoration::Waveform(bins)
                            }
                            Err(_) => ClipDecoration::Unavailable,
                        };
                        if self.decorations.len() >= 128
                            && let Some(old) = self.decorations.keys().next().copied()
                        {
                            self.decorations.remove(&old);
                        }
                        self.decorations.insert(id, decoration);
                    }
                }
                Event::Relink(project, original, result) => {
                    self.importing = false;
                    if project != self.project.project_uuid
                        || self.project.asset(original.id).ok() != Some(original.as_ref())
                    {
                        self.status = self.tr("stale-result");
                        continue;
                    }
                    if let Some(asset) = self.report(result) {
                        if asset.kind != original.kind {
                            self.status = self.tr("relink-kind-mismatch");
                            continue;
                        }
                        let id = original.id;
                        self.edit(|p| {
                            let old = p
                                .assets
                                .iter_mut()
                                .find(|a| a.id == id)
                                .ok_or("missing-asset")?;
                            *old = MediaAsset { id, ..asset };
                            Ok(())
                        });
                        self.decorations.remove(&id);
                    }
                }
            }
        }
        self.poll_transport(ctx);
        self.poll_package(ctx);
        while let Ok((id, result)) = self.preview_worker.receiver.try_recv() {
            if id != self.preview_ticket {
                continue;
            }
            self.preview.pending = false;
            if let Some(frame) = self.report(result) {
                self.install_preview(ctx, frame);
            } else {
                self.cancel_preview_transform();
            }
        }
        if self.preview_dirty
            && !self.transport.active()
            && !(self.preview.drag.is_some() && self.preview.pending)
        {
            self.preview_dirty = false;
            if !self.preview.retain && self.preview.drag.is_none() {
                self.texture = None;
            }
            if self.tools.is_some()
                && let Some(plan) = self.report(RenderPlan::new(&self.project, &self.storage))
            {
                self.preview.pending = true;
                self.preview_ticket = self.preview_worker.request(PreviewRequest {
                    project: self.project.project_uuid,
                    selected: self.selected,
                    plan,
                    storage: self.storage.clone(),
                    ffmpeg: self.settings.ffmpeg.clone(),
                    frame: self.playhead,
                    width: self.settings.preview_width,
                    limit_mb: self.settings.cache_limit_mb / 2,
                });
            } else {
                self.preview_worker.cancel();
                self.preview_ticket = 0;
                self.preview.pending = false;
            }
        }
        let finished = self.export.as_mut().and_then(|job| {
            for value in job.progress.try_iter() {
                job.value = value;
            }
            job.receiver.try_recv().ok()
        });
        if let Some(result) = finished {
            self.export = None;
            if self.report(result).is_some() {
                self.status = self.tr("export-complete");
            }
        }
        if let Some(child) = &mut self.playback {
            match child.try_wait() {
                Ok(Some(status)) => {
                    self.stop_audio();
                    if !status.success() {
                        self.status = self.tr("audio-playback-failed");
                    }
                }
                Ok(None) => ctx.request_repaint_after(Duration::from_millis(200)),
                Err(e) => {
                    self.status = e.to_string();
                    self.stop_audio();
                }
            }
        }
    }
    pub fn persist_settings(&mut self) {
        let result = self.settings.save(&self.storage);
        self.report(result);
    }
    pub fn new_project(&mut self) {
        self.finish_property_edit();
        self.resume_after_save = false;
        self.cancel_preview_transform();
        self.file_picker = None;
        self.stop_audio();
        self.project = Project::default();
        self.saved = self.project.clone();
        self.project_path = None;
        self.history = History::default();
        self.selected = None;
        self.track = 0;
        self.playhead = 0;
        self.voice_text.clear();
        self.changed();
    }
}
impl Drop for App {
    fn drop(&mut self) {
        self.cancel_timeline();
        self.stop_audio();
        self.preview_worker.cancel();
        if let Some(job) = &self.export {
            job.cancel.store(true, Ordering::Relaxed);
        }
        if let Some(job) = &self.package {
            job.cancel.store(true, Ordering::Relaxed);
        }
    }
}
