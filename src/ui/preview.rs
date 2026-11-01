use crate::{
    app::App,
    media::preview::{PreviewFrame, PreviewObject},
    project::{ClipEffects, Id},
};
use eframe::egui;

#[derive(Default)]
pub struct PreviewState {
    pub rect: Option<egui::Rect>,
    pub scene: Option<Scene>,
    pub drag: Option<TransformDrag>,
    pub pending: bool,
    pub retain: bool,
    pick: Option<PendingPick>,
}
struct PendingPick {
    origin: egui::Pos2,
    latest: egui::Pos2,
    display_size: egui::Vec2,
    released: bool,
}
pub struct Scene {
    project: Id,
    frame: u64,
    objects: Vec<PreviewObject>,
    planes: Option<Planes>,
}
struct Planes {
    id: Id,
    behind: egui::TextureHandle,
    object: egui::TextureHandle,
    front: egui::TextureHandle,
}
pub struct TransformDrag {
    id: Id,
    before: ClipEffects,
    origin: egui::Pos2,
    display_size: egui::Vec2,
}
impl App {
    pub fn install_preview(&mut self, ctx: &egui::Context, frame: PreviewFrame) {
        let upload = |name, image: &image::RgbaImage| {
            ctx.load_texture(
                name,
                egui::ColorImage::from_rgba_unmultiplied(
                    [image.width() as usize, image.height() as usize],
                    image.as_raw(),
                ),
                egui::TextureOptions::LINEAR,
            )
        };
        self.texture = Some(upload("preview", &frame.image));
        let planes = frame.planes.map(|p| Planes {
            id: p.id,
            behind: upload("preview-behind", &p.behind),
            object: upload("preview-object", &p.object),
            front: upload("preview-front", &p.front),
        });
        if self
            .preview
            .drag
            .as_ref()
            .is_some_and(|d| planes.as_ref().is_some_and(|p| p.id == d.id))
        {
            self.preview_dirty = false;
        }
        self.preview.scene = Some(Scene {
            project: frame.project,
            frame: frame.frame,
            objects: frame.objects,
            planes,
        });
        self.preview.retain = false;
    }
    fn preview_bounds(&self, object: &PreviewObject, rect: egui::Rect) -> Option<egui::Rect> {
        let clip = self.project.clip(object.id)?;
        let track = &self.project.tracks[self.project.track_of(object.id)?];
        if !clip.active(self.playhead)
            || !track.enabled
            || !track.video_visible
            || clip
                .effects
                .visual_alpha(self.playhead - clip.timeline_start, clip.duration)
                <= 0.0
        {
            return None;
        }
        let size = [self.project.width as f64, self.project.height as f64];
        let b = object.transformed(size, &clip.effects, self.playhead - clip.timeline_start);
        let point = |x: f64, y: f64| {
            rect.min
                + egui::vec2(
                    (x / size[0]) as f32 * rect.width(),
                    (y / size[1]) as f32 * rect.height(),
                )
        };
        Some(egui::Rect::from_min_max(
            point(b[0], b[1]),
            point(b[2], b[3]),
        ))
    }
    fn preview_hit(&self, position: egui::Pos2, rect: egui::Rect) -> Option<Id> {
        if !rect.contains(position) {
            return None;
        }
        let scene = self.preview.scene.as_ref()?;
        if scene.project != self.project.project_uuid || scene.frame != self.playhead {
            return None;
        }
        scene
            .objects
            .iter()
            .rev()
            .find(|o| {
                self.preview_bounds(o, rect)
                    .is_some_and(|r| r.contains(position))
            })
            .map(|o| o.id)
    }
    fn preview_can_edit(&self, id: Id) -> bool {
        self.project
            .track_of(id)
            .is_some_and(|index| !self.project.tracks[index].locked)
    }
    pub fn finish_preview_transform(&mut self) {
        self.preview.pick = None;
        let Some(drag) = self.preview.drag.take() else {
            return;
        };
        let Some(clip) = self.project.clip_mut(drag.id) else {
            return;
        };
        let after = clip.effects.clone();
        if after == drag.before {
            return;
        }
        clip.effects = drag.before;
        let result = self.history.edit(&mut self.project, |project| {
            project.clip_mut(drag.id).ok_or("missing-clip")?.effects = after;
            Ok(())
        });
        if self.report(result).is_some() {
            self.cancel_timeline();
            self.revision += 1;
            let ready = self
                .preview
                .scene
                .as_ref()
                .is_some_and(|s| s.planes.as_ref().is_some_and(|p| p.id == drag.id));
            if !ready {
                self.preview.scene = None;
                self.preview_dirty = true;
                self.preview.retain = true;
            }
        }
    }
    pub fn cancel_preview_transform(&mut self) {
        self.preview.pick = None;
        if let Some(drag) = self.preview.drag.take() {
            if let Some(clip) = self.project.clip_mut(drag.id) {
                clip.effects = drag.before;
            }
            self.preview_dirty = true;
            self.preview.retain = true;
        }
    }
    fn preview_transform_updated(&mut self, id: Id, ctx: &egui::Context) {
        let ready = self
            .preview
            .scene
            .as_ref()
            .is_some_and(|s| s.planes.as_ref().is_some_and(|p| p.id == id));
        if !ready {
            self.preview_dirty = true;
            self.preview.retain = true;
        }
        ctx.request_repaint();
    }
    pub fn preview_interaction(&mut self, ui: &mut egui::Ui, rect: egui::Rect) {
        self.preview.rect = Some(rect);
        let ctx = ui.ctx().clone();
        let response = ui
            .interact(
                rect,
                ui.id().with("preview-transform"),
                egui::Sense::click_and_drag(),
            )
            .on_hover_text(self.tr("preview-transform-hint"));
        let primary = egui::PointerButton::Primary;
        if (response.is_pointer_button_down_on() || response.clicked_by(primary))
            && ctx.input(|i| i.pointer.button_pressed(primary))
        {
            self.finish_preview_transform();
            self.pause_timeline();
            if (self.tools.is_some() || self.preview.scene.is_some() || self.preview.pending)
                && let Some(point) = response.interact_pointer_pos()
            {
                self.preview.pick = Some(PendingPick {
                    origin: point,
                    latest: point,
                    display_size: rect.size(),
                    released: false,
                });
                self.preview.retain = true;
                if self.preview.scene.is_none() {
                    self.preview_dirty = true;
                }
            }
        }
        if let Some(pick) = &mut self.preview.pick
            && !pick.released
        {
            if let Some(point) = ctx.input(|i| i.pointer.latest_pos()) {
                pick.latest = point;
            }
            pick.released = !ctx.input(|i| i.pointer.primary_down());
        }
        let scene_ready =
            self.preview.scene.as_ref().is_some_and(|s| {
                s.project == self.project.project_uuid && s.frame == self.playhead
            });
        let mut resolved_position = None;
        if scene_ready && let Some(pick) = self.preview.pick.take() {
            if let Some(id) = self.preview_hit(pick.origin, rect) {
                self.select(id);
                if self.preview_can_edit(id) {
                    self.preview.drag = self.project.clip(id).map(|c| TransformDrag {
                        id,
                        before: c.effects.clone(),
                        origin: pick.origin,
                        display_size: pick.display_size,
                    });
                    resolved_position = Some(pick.latest);
                }
            } else {
                self.selected = None;
            }
        }
        let drag_position = resolved_position.or_else(|| {
            (response.dragged_by(primary) || response.drag_stopped_by(primary))
                .then(|| response.interact_pointer_pos())
                .flatten()
        });
        if let Some(drag) = &self.preview.drag
            && let Some(point) = drag_position
        {
            let id = drag.id;
            let delta = point - drag.origin;
            let x = drag.before.x
                + delta.x as f64 * self.project.width as f64 / drag.display_size.x.max(1.0) as f64;
            let y = drag.before.y
                + delta.y as f64 * self.project.height as f64 / drag.display_size.y.max(1.0) as f64;
            if self.preview_can_edit(id)
                && let Some(clip) = self.project.clip_mut(id)
            {
                clip.effects.x = x.clamp(-15360.0, 15360.0);
                clip.effects.y = y.clamp(-8640.0, 8640.0);
                self.preview_transform_updated(id, &ctx);
            }
        }
        if self.preview.drag.is_some() && !ctx.input(|i| i.pointer.primary_down()) {
            self.finish_preview_transform();
        }
        if response.hovered() {
            let wheel = ctx.input(|i| i.raw_scroll_delta.y);
            if wheel != 0.0
                && self.preview.drag.is_none()
                && let Some(point) = response.hover_pos()
                && let Some(id) = self.preview_hit(point, rect)
                && self.preview_can_edit(id)
            {
                self.pause_timeline();
                self.select(id);
                let before = self.project.clip(id).map(|c| c.effects.clone());
                if let Some(before) = before {
                    let size = [self.project.width as f64, self.project.height as f64];
                    let Some(clip) = self.project.clip_mut(id) else {
                        return;
                    };
                    let scale = (before.scale * (wheel as f64 * 0.002).exp()).clamp(0.05, 4.0);
                    let ratio = scale / before.scale;
                    let x =
                        (point.x - rect.center().x) as f64 * size[0] / rect.width().max(1.0) as f64;
                    let y = (point.y - rect.center().y) as f64 * size[1]
                        / rect.height().max(1.0) as f64;
                    let slide = before.slide_offset(self.playhead - clip.timeline_start, size[0]);
                    clip.effects.scale = scale;
                    clip.effects.x =
                        (x - (x - before.x - slide) * ratio - slide).clamp(-15360.0, 15360.0);
                    clip.effects.y = (y - (y - before.y) * ratio).clamp(-8640.0, 8640.0);
                    self.preview.drag = Some(TransformDrag {
                        id,
                        before,
                        origin: point,
                        display_size: rect.size(),
                    });
                    self.preview_transform_updated(id, &ctx);
                    self.finish_preview_transform();
                    ctx.input_mut(|i| {
                        i.raw_scroll_delta = egui::Vec2::ZERO;
                        i.smooth_scroll_delta = egui::Vec2::ZERO;
                    });
                }
            }
        }
        if let Some(scene) = &self.preview.scene
            && scene.project == self.project.project_uuid
            && scene.frame == self.playhead
            && !self.transport.active()
            && let Some(planes) = &scene.planes
            && let Some(clip) = self.project.clip(planes.id)
        {
            let painter = ui.painter().with_clip_rect(rect);
            let uv = egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0));
            let relative = self.playhead.saturating_sub(clip.timeline_start);
            let offset = egui::vec2(
                ((clip.effects.x
                    + clip
                        .effects
                        .slide_offset(relative, self.project.width as f64))
                    / self.project.width as f64) as f32
                    * rect.width(),
                (clip.effects.y / self.project.height as f64) as f32 * rect.height(),
            );
            let object = egui::Rect::from_center_size(
                rect.center() + offset,
                rect.size() * clip.effects.scale as f32,
            );
            painter.image(planes.behind.id(), rect, uv, egui::Color32::WHITE);
            painter.image(planes.object.id(), object, uv, egui::Color32::WHITE);
            painter.image(planes.front.id(), rect, uv, egui::Color32::WHITE);
        }
        if let Some(scene) = &self.preview.scene
            && scene.project == self.project.project_uuid
            && scene.frame == self.playhead
            && let Some(object) = scene.objects.iter().find(|o| Some(o.id) == self.selected)
            && let Some(bounds) = self.preview_bounds(object, rect)
        {
            let color = if self.preview_can_edit(object.id) {
                ui.visuals().selection.stroke.color
            } else {
                ui.visuals().weak_text_color()
            };
            ui.painter().with_clip_rect(rect).rect_stroke(
                bounds.expand(2.0),
                0.0,
                egui::Stroke::new(1.5_f32, color),
                egui::StrokeKind::Inside,
            );
        }
    }
}
