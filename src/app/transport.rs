use super::App;
use crate::{
    media::{
        playback::{ProxyRequest, Session, VideoFrame},
        render_plan::RenderPlan,
    },
    tasks::ProxyWorker,
};
use eframe::egui;
use std::{
    sync::{Arc, atomic::AtomicU32},
    time::Duration,
};

pub struct Transport {
    pub worker: ProxyWorker,
    pub preparing: Option<u64>,
    pub progress: Arc<AtomicU32>,
    pub session: Option<Session>,
    pub skipped: u64,
    pending: Option<VideoFrame>,
    start: u64,
    displayed_skips: u64,
    first_frame_ready: Option<u64>,
}
impl Transport {
    pub fn new(ctx: egui::Context) -> Self {
        Self {
            worker: ProxyWorker::new(ctx),
            preparing: None,
            progress: Arc::new(AtomicU32::new(0)),
            session: None,
            skipped: 0,
            pending: None,
            start: 0,
            displayed_skips: 0,
            first_frame_ready: None,
        }
    }
    pub fn active(&self) -> bool {
        self.preparing.is_some() || self.session.is_some()
    }
}
impl App {
    pub fn cancel_timeline(&mut self) {
        self.transport.worker.cancel();
        self.transport.preparing = None;
        self.transport.session = None;
        self.transport.pending = None;
        self.transport.first_frame_ready = None;
    }
    pub fn pause_timeline(&mut self) {
        if let Some(session) = &self.transport.session {
            self.playhead = session.frame().min(self.project.duration());
        }
        if self.transport.active() {
            self.cancel_timeline();
            self.preview_dirty = true;
        }
    }
    pub fn stop_timeline(&mut self) {
        self.finish_preview_transform();
        self.cancel_timeline();
        self.playhead = 0;
        self.preview_dirty = true;
    }
    pub fn seek_timeline(&mut self, frame: u64, ctx: &egui::Context) {
        self.finish_preview_transform();
        let resume = self.transport.active();
        self.cancel_timeline();
        self.playhead = frame.min(crate::project::MAX_FRAME);
        self.preview_dirty = true;
        if resume && self.playhead < self.project.duration() {
            self.start_timeline(ctx);
        }
    }
    pub fn toggle_timeline(&mut self, ctx: &egui::Context) {
        if self.transport.active() {
            self.pause_timeline();
        } else {
            self.start_timeline(ctx);
        }
    }
    pub fn start_timeline(&mut self, ctx: &egui::Context) {
        self.finish_preview_transform();
        self.preview.scene = None;
        self.preview.pending = false;
        self.cancel_timeline();
        self.stop_audio();
        let Some(tools) = &self.tools else {
            self.status = self.tr("ffmpeg-missing");
            return;
        };
        if !tools.supports_encoder("mpeg4")
            || !tools.supports_encoder("pcm_s16le")
            || !tools.supports_muxer("nut")
        {
            self.status = self.tr("proxy-encoder-unavailable");
            return;
        }
        if self
            .project
            .tracks
            .iter()
            .all(|track| track.clips.is_empty())
        {
            self.status = self.tr("empty-project");
            return;
        }
        let Some(plan) = self.report(RenderPlan::new(&self.project, &self.storage)) else {
            return;
        };
        if self.playhead >= plan.duration {
            self.playhead = 0;
        }
        self.transport.start = self.playhead;
        self.transport.skipped = 0;
        self.transport.displayed_skips = 0;
        self.transport.progress = Arc::new(AtomicU32::new(0));
        self.transport.preparing = Some(self.transport.worker.request(ProxyRequest {
            plan,
            storage: self.storage.clone(),
            ffmpeg: self.settings.ffmpeg.clone(),
            width: self.settings.preview_width,
            limit_mb: self.settings.cache_limit_mb / 2,
            progress: self.transport.progress.clone(),
        }));
        self.preview_worker.cancel();
        self.preview_ticket = 0;
        self.preview_dirty = false;
        ctx.request_repaint();
    }
    /// Eframe paints after update returns. Acknowledge on the next update, so the
    /// first image's layout, upload and paint are not charged to playback time.
    pub fn present_timeline(&self, ctx: &egui::Context) {
        if let Some(frame) = self.transport.first_frame_ready {
            if ctx.cumulative_frame_nr() > frame {
                if let Some(session) = &self.transport.session {
                    session.presented();
                }
            } else {
                ctx.request_repaint();
            }
        }
    }
    pub fn poll_transport(&mut self, ctx: &egui::Context) {
        while let Ok((ticket, result)) = self.transport.worker.receiver.try_recv() {
            if self.transport.preparing != Some(ticket) {
                continue;
            }
            self.transport.preparing = None;
            if let Some(proxy) = self.report(result) {
                let device = self
                    .settings
                    .preview_audio
                    .then(|| self.settings.audio_device.clone());
                self.transport.session = Some(Session::start(
                    proxy,
                    self.settings.ffmpeg.clone(),
                    self.transport.start,
                    device,
                    ctx.clone(),
                ));
            } else {
                self.preview_dirty = true;
            }
        }
        if self.transport.preparing.is_some() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        let Some(session) = self.transport.session.as_ref() else {
            return;
        };
        if let Ok(error) = session.errors.try_recv() {
            self.status = self.local_error(&error);
            self.pause_timeline();
            return;
        }
        self.playhead = session.frame();
        let mut latest = None;
        for _ in 0..8 {
            let next = self
                .transport
                .pending
                .take()
                .or_else(|| session.frames.try_recv().ok());
            let Some(frame) = next else {
                break;
            };
            if frame.frame > self.playhead {
                self.transport.pending = Some(frame);
                break;
            }
            if latest.is_some() {
                self.transport.displayed_skips += 1;
            }
            latest = Some(frame);
        }
        if let Some(frame) = latest {
            let image = egui::ColorImage::from_rgba_unmultiplied(
                [session.proxy.width as usize, session.proxy.height as usize],
                &frame.rgba,
            );
            if let Some(texture) = &mut self.texture {
                texture.set(image, egui::TextureOptions::LINEAR);
            } else {
                self.texture =
                    Some(ctx.load_texture("preview", image, egui::TextureOptions::LINEAR));
            }
            self.transport
                .first_frame_ready
                .get_or_insert(ctx.cumulative_frame_nr());
        }
        self.transport.skipped = session.skipped() + self.transport.displayed_skips;
        if session.ended() {
            // Clip ranges are half-open: the end is after the final picture's start.
            // Keep that picture instead of clearing it for another asynchronous decode.
            self.playhead = self.project.duration();
            self.cancel_timeline();
            self.preview_dirty = false;
        } else {
            ctx.request_repaint_after(Duration::from_millis(16));
        }
    }
}
