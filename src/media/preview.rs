//! Bounded, disposable editor planes. All pixels use the shared RenderPlan compiler.
use super::{cache, render_plan::RenderPlan, text};
use crate::{
    Result,
    project::{ClipEffects, Id, TransitionKind},
    tasks::PreviewRequest,
};
use image::RgbaImage;

#[derive(Clone, Debug)]
pub struct PreviewObject {
    pub id: Id,
    /// Untransformed bounds in project pixels, including fit-to-canvas padding.
    pub bounds: [f64; 4],
}
impl PreviewObject {
    pub fn transformed(&self, size: [f64; 2], effects: &ClipEffects, relative: u64) -> [f64; 4] {
        let slide = effects.slide_offset(relative, size[0]);
        let x = |v: f64| (v - size[0] / 2.0) * effects.scale + size[0] / 2.0 + effects.x + slide;
        let y = |v: f64| (v - size[1] / 2.0) * effects.scale + size[1] / 2.0 + effects.y;
        [
            x(self.bounds[0]),
            y(self.bounds[1]),
            x(self.bounds[2]),
            y(self.bounds[3]),
        ]
    }
}
pub struct EditPlanes {
    pub id: Id,
    pub behind: RgbaImage,
    pub object: RgbaImage,
    pub front: RgbaImage,
}
pub struct PreviewFrame {
    pub image: RgbaImage,
    pub objects: Vec<PreviewObject>,
    pub planes: Option<EditPlanes>,
    pub project: Id,
    pub frame: u64,
}
pub fn objects(plan: &RenderPlan, frame: u64, width: u32) -> Result<Vec<PreviewObject>> {
    let (w, h) = plan.preview_size(width);
    let mut objects = Vec::new();
    for clip in &plan.clips {
        if !clip.visual || frame < clip.start || frame >= clip.start + clip.duration {
            continue;
        }
        let bounds = if let Some(style) = &clip.text {
            let image = text::rasterize(style, w, h, plan.width)?;
            let mut bounds = [image.width(), image.height(), 0, 0];
            for (x, y, pixel) in image.enumerate_pixels() {
                if pixel[3] > 0 {
                    bounds[0] = bounds[0].min(x);
                    bounds[1] = bounds[1].min(y);
                    bounds[2] = bounds[2].max(x + 1);
                    bounds[3] = bounds[3].max(y + 1);
                }
            }
            if bounds[2] == 0 {
                continue;
            }
            [
                bounds[0] as f64 * plan.width as f64 / image.width() as f64,
                bounds[1] as f64 * plan.height as f64 / image.height() as f64,
                bounds[2] as f64 * plan.width as f64 / image.width() as f64,
                bounds[3] as f64 * plan.height as f64 / image.height() as f64,
            ]
        } else {
            let [sw, sh] = if let Some(sprite) = &clip.sprite {
                let doc = crate::sprites::psd::load(&sprite.settings.psd)?;
                [doc.width, doc.height]
            } else {
                clip.source_size
            };
            if sw == 0 || sh == 0 {
                continue;
            }
            let fit = (plan.width as f64 / sw as f64).min(plan.height as f64 / sh as f64);
            let x = (plan.width as f64 - sw as f64 * fit) / 2.0;
            let y = (plan.height as f64 - sh as f64 * fit) / 2.0;
            [x, y, plan.width as f64 - x, plan.height as f64 - y]
        };
        objects.push(PreviewObject {
            id: clip.id,
            bounds,
        });
    }
    Ok(objects)
}
pub fn render(request: PreviewRequest, cancelled: &dyn Fn() -> bool) -> Result<PreviewFrame> {
    let render =
        |plan: &RenderPlan| {
            if cancelled() {
                return Err("cancelled".into());
            }
            if !plan.clips.iter().any(|c| {
                c.visual && c.start <= request.frame && request.frame < c.start + c.duration
            }) {
                let (w, h) = plan.preview_size(request.width);
                return Ok(RgbaImage::from_pixel(
                    w,
                    h,
                    image::Rgba([0, 0, 0, if plan.transparent { 0 } else { 255 }]),
                ));
            }
            cache::preview(
                plan,
                &request.storage,
                &request.ffmpeg,
                request.frame,
                request.width,
                request.limit_mb,
                cancelled,
            )
        };
    let objects = objects(&request.plan, request.frame, request.width)?;
    let image = render(&request.plan)?;
    let mut planes = None;
    if let Some(id) = request
        .selected
        .filter(|id| objects.iter().any(|o| o.id == *id))
    {
        let first = request
            .plan
            .clips
            .iter()
            .position(|c| c.id == id)
            .ok_or("missing-clip")?;
        let last = request
            .plan
            .clips
            .iter()
            .rposition(|c| c.id == id)
            .ok_or("missing-clip")?;
        let mut behind = request.plan.clone();
        behind.clips.truncate(first);
        let mut front = request.plan.clone();
        front.clips.drain(..=last);
        front.transparent = true;
        let mut object = request.plan.clone();
        object.clips.retain(|c| c.id == id && c.visual);
        object.transparent = true;
        for c in &mut object.clips {
            c.effects.x = 0.0;
            c.effects.y = 0.0;
            c.effects.scale = 1.0;
            if c.effects.transition == TransitionKind::SlideLeft {
                c.effects.transition = TransitionKind::None;
            }
        }
        planes = Some(EditPlanes {
            id,
            behind: render(&behind)?,
            object: render(&object)?,
            front: render(&front)?,
        });
    }
    Ok(PreviewFrame {
        image,
        objects,
        planes,
        project: request.project,
        frame: request.frame,
    })
}
