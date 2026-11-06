//! Lazy, bounded thumbnails shared by every animation-layer selector.
use crate::{
    i18n::I18n,
    sprites::psd::{Document, Node},
    tasks::LatestWorker,
};
use eframe::egui;
use std::{
    collections::{BTreeSet, VecDeque},
    sync::Arc,
};

const CACHE_ENTRIES: usize = 128;
const BATCH_SIZE: usize = 8;
type Images = Vec<(String, crate::Result<image::RgbaImage>)>;
type Texture = crate::Result<egui::TextureHandle>;
struct Request {
    document: Arc<Document>,
    keys: Vec<String>,
}

#[derive(Default)]
pub(super) struct LayerThumbnails {
    document: Option<Arc<Document>>,
    textures: VecDeque<(String, Texture)>,
    worker: Option<LatestWorker<Request, Images>>,
    ticket: u64,
    pending: bool,
    wanted: BTreeSet<String>,
}
impl LayerThumbnails {
    pub fn begin_frame(&mut self, document: Option<&Arc<Document>>, ctx: &egui::Context) {
        let same = match (&self.document, document) {
            (Some(old), Some(new)) => Arc::ptr_eq(old, new),
            (None, None) => true,
            _ => false,
        };
        if !same {
            if let Some(worker) = &self.worker {
                worker.cancel();
            }
            self.document = document.cloned();
            self.textures.clear();
            self.ticket = 0;
            self.pending = false;
        }
        self.wanted.clear();
        if let Some(worker) = &self.worker {
            while let Ok((ticket, result)) = worker.receiver.try_recv() {
                if ticket != self.ticket {
                    continue;
                }
                self.pending = false;
                if let Ok(images) = result {
                    for (key, image) in images {
                        let texture = image.map(|image| {
                            ctx.load_texture(
                                format!("sprite-layer-{key}"),
                                egui::ColorImage::from_rgba_unmultiplied(
                                    [image.width() as usize, image.height() as usize],
                                    image.as_raw(),
                                ),
                                egui::TextureOptions::LINEAR,
                            )
                        });
                        self.textures.retain(|(existing, _)| existing != &key);
                        while self.textures.len() >= CACHE_ENTRIES {
                            self.textures.pop_front();
                        }
                        self.textures.push_back((key, texture));
                    }
                }
            }
        }
    }
    pub fn show(&mut self, ui: &mut egui::Ui, node: &Node, i18n: &I18n) -> egui::Response {
        let (rect, response) = ui.allocate_exact_size(egui::vec2(40.0, 40.0), egui::Sense::click());
        if !ui.is_rect_visible(rect) {
            return response;
        }
        checkerboard(ui, rect);
        let entry = self
            .textures
            .iter()
            .position(|(key, _)| key == &node.key)
            .and_then(|index| self.textures.remove(index));
        if let Some((key, texture)) = entry {
            match &texture {
                Ok(texture) => {
                    let size = texture.size_vec2();
                    let scale = ((rect.width() - 4.0) / size.x).min((rect.height() - 4.0) / size.y);
                    egui::Image::new(texture).paint_at(
                        ui,
                        egui::Rect::from_center_size(rect.center(), size * scale),
                    );
                }
                Err(_) => {
                    ui.painter().text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        "!",
                        egui::FontId::proportional(16.0),
                        ui.visuals().error_fg_color,
                    );
                }
            }
            let response = match &texture {
                Ok(_) => response.on_hover_text(&node.name),
                Err(error) => response.on_hover_text(format!(
                    "{}: {error}",
                    i18n.text("sprite-layer-preview-failed")
                )),
            };
            self.textures.push_back((key, texture));
            response
        } else {
            self.wanted.insert(node.key.clone());
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "…",
                egui::FontId::proportional(16.0),
                ui.visuals().weak_text_color(),
            );
            response.on_hover_text(i18n.text("sprite-layer-preview-loading"))
        }
    }
    pub fn finish_frame(&mut self, ctx: &egui::Context) {
        if self.pending || self.wanted.is_empty() {
            return;
        }
        let Some(document) = self.document.clone() else {
            return;
        };
        if self.worker.is_none() {
            self.worker = Some(LatestWorker::with_processor(
                ctx.clone(),
                |request: Request, cancelled| {
                    let mut images = Vec::new();
                    for key in request.keys {
                        if cancelled() {
                            return Err("cancelled".into());
                        }
                        let image = request.document.element_thumbnail(&key, cancelled);
                        images.push((key, image));
                    }
                    Ok(images)
                },
            ));
        }
        self.ticket = self
            .worker
            .as_ref()
            .expect("initialized layer thumbnail worker")
            .request(Request {
                document,
                keys: self.wanted.iter().take(BATCH_SIZE).cloned().collect(),
            });
        self.pending = true;
    }
}

fn checkerboard(ui: &egui::Ui, rect: egui::Rect) {
    let colors = if ui.visuals().dark_mode {
        [egui::Color32::from_gray(45), egui::Color32::from_gray(70)]
    } else {
        [egui::Color32::from_gray(210), egui::Color32::from_gray(240)]
    };
    let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
    for y in 0..8 {
        for x in 0..8 {
            let cell = egui::Rect::from_min_size(
                rect.min + egui::vec2(x as f32 * 5.0, y as f32 * 5.0),
                egui::vec2(5.0, 5.0),
            );
            painter.rect_filled(cell, 0.0, colors[(x + y) % 2]);
        }
    }
}
