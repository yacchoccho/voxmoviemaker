use crate::{
    app::{App, DragClip},
    project::{commands, *},
};
use eframe::egui;
pub const LABEL_WIDTH: f32 = 112.0;
pub const RULER_HEIGHT: f32 = 26.0;
pub const ROW_HEIGHT: f32 = 48.0;

enum ClipAction {
    Split(Id),
    Delete(Id),
    Crossfade(Id),
}

/// Canvas rect already includes both ScrollArea offsets, so hit testing and drawing share one origin.
pub fn position(
    point: egui::Pos2,
    canvas: egui::Rect,
    fps: FrameRate,
    pixels_per_second: f32,
    tracks: usize,
) -> (usize, u64) {
    let seconds = ((point.x - canvas.left() - LABEL_WIDTH) / pixels_per_second).max(0.0);
    let frame = (seconds as f64 * fps.numerator as f64 / fps.denominator as f64)
        .round()
        .clamp(0.0, MAX_FRAME as f64) as u64;
    let track = (((point.y - canvas.top() - RULER_HEIGHT) / ROW_HEIGHT)
        .floor()
        .max(0.0) as usize)
        .min(tracks);
    (track, frame)
}
impl App {
    pub fn timeline(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.horizontal_wrapped(|ui| {
            ui.strong(self.tr("timeline"));
            if ui.button(self.tr("add-media")).clicked() {
                self.request_path(crate::app::PathMode::Import);
            }
            if ui.button(self.tr("add-subtitle")).clicked() {
                self.insert_text(true, false);
            }
            if ui.button(self.tr("add-text")).clicked() {
                self.insert_text(false, false);
            }
            if ui.button(self.tr("add-layer")).clicked() {
                self.edit(|p| {
                    p.tracks.push(Track::new(p.tracks.len() + 1));
                    Ok(())
                });
            }
            if ui.button(self.tr("remove-layer")).clicked() {
                let track = self.track;
                self.edit(|p| {
                    let t = p.tracks.get(track).ok_or("invalid-track")?;
                    if t.locked {
                        return Err("track-locked".into());
                    }
                    if !t.clips.is_empty() {
                        return Err("layer-not-empty".into());
                    }
                    if p.tracks.len() <= 1 {
                        return Err("last-layer".into());
                    }
                    p.tracks.remove(track);
                    Ok(())
                });
            }
            if ui.button(self.tr("layer-up")).clicked() && self.track > 0 {
                let index = self.track;
                self.edit(|p| {
                    p.tracks.swap(index, index - 1);
                    Ok(())
                });
                self.track -= 1;
            }
            if ui.button(self.tr("layer-down")).clicked()
                && self.track + 1 < self.project.tracks.len()
            {
                let index = self.track;
                self.edit(|p| {
                    p.tracks.swap(index, index + 1);
                    Ok(())
                });
                self.track += 1;
            }
            let snap = self.tr("snap");
            ui.checkbox(&mut self.snap, snap);
            let zoom = self.tr("zoom");
            ui.add(
                egui::Slider::new(&mut self.pixels_per_second, 10.0..=500.0)
                    .logarithmic(true)
                    .text(zoom),
            );
        });
        ui.small(self.tr("layer-order-hint"));
        let duration = self
            .project
            .frame_rate
            .seconds(self.project.duration())
            .max(30.0) as f32
            + 10.0;
        let width = (duration * self.pixels_per_second + LABEL_WIDTH).max(ui.available_width());
        let height = ((self.project.tracks.len() + 1) as f32 * ROW_HEIGHT + RULER_HEIGHT)
            .max(ui.available_height());
        let mut select = None;
        let mut decorate = Vec::new();
        let mut move_to = None;
        let mut action = None;
        let mut make_subtitle = false;
        egui::ScrollArea::both()
            .id_salt("timeline-scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let (canvas, response) = ui
                    .allocate_exact_size(egui::vec2(width, height), egui::Sense::click_and_drag());
                let viewport = ui.clip_rect().intersect(canvas);
                self.timeline_rect = Some(viewport);
                let painter = ui.painter_at(canvas);
                let fps = self.project.frame_rate;
                let pixels_per_second = self.pixels_per_second;
                let x_for = |frame: u64| {
                    canvas.left() + LABEL_WIDTH + fps.seconds(frame) as f32 * pixels_per_second
                };
                if let Some(id) = self.reveal_clip.take()
                    && let Some(clip) = self.project.clip(id)
                    && let Some(index) = self.project.track_of(id)
                {
                    let top = canvas.top() + RULER_HEIGHT + index as f32 * ROW_HEIGHT;
                    let target = egui::Rect::from_min_size(
                        egui::pos2(x_for(clip.timeline_start), top),
                        egui::vec2(100.0, ROW_HEIGHT),
                    );
                    ui.scroll_to_rect(target, Some(egui::Align::Center));
                }
                let second_step = if self.pixels_per_second < 30.0 { 5 } else { 1 };
                let first_second = ((viewport.left() - canvas.left() - LABEL_WIDTH)
                    / self.pixels_per_second)
                    .floor()
                    .max(0.0) as u64;
                let last_second = ((viewport.right() - canvas.left() - LABEL_WIDTH)
                    / self.pixels_per_second)
                    .ceil()
                    .max(0.0) as u64;
                for second in (first_second..=last_second).filter(|s| s % second_step == 0) {
                    let x = canvas.left() + LABEL_WIDTH + second as f32 * self.pixels_per_second;
                    painter.line_segment(
                        [egui::pos2(x, canvas.top()), egui::pos2(x, canvas.bottom())],
                        egui::Stroke::new(
                            1.0_f32,
                            ui.visuals().widgets.noninteractive.bg_stroke.color,
                        ),
                    );
                    painter.text(
                        egui::pos2(x + 4.0, canvas.top() + 5.0),
                        egui::Align2::LEFT_TOP,
                        format!("{second}s"),
                        egui::FontId::proportional(12.0),
                        ui.visuals().text_color(),
                    );
                }
                for (index, track) in self.project.tracks.iter().enumerate() {
                    let top = canvas.top() + RULER_HEIGHT + index as f32 * ROW_HEIGHT;
                    let row = egui::Rect::from_min_size(
                        egui::pos2(canvas.left(), top),
                        egui::vec2(width, ROW_HEIGHT),
                    );
                    if !viewport.intersects(row) {
                        continue;
                    }
                    if index == self.track {
                        painter.rect_filled(
                            row,
                            0.0,
                            ui.visuals().selection.bg_fill.gamma_multiply(0.22),
                        );
                    }
                    let label_rect =
                        egui::Rect::from_min_size(row.min, egui::vec2(LABEL_WIDTH, ROW_HEIGHT));
                    painter.rect_filled(label_rect, 0.0, ui.visuals().panel_fill);
                    painter.text(
                        label_rect.left_center() + egui::vec2(7.0, 0.0),
                        egui::Align2::LEFT_CENTER,
                        format!("{} · {}", index + 1, track.name),
                        egui::FontId::proportional(13.0),
                        if track.enabled {
                            ui.visuals().text_color()
                        } else {
                            ui.visuals().weak_text_color()
                        },
                    );
                    if ui
                        .interact(
                            label_rect,
                            egui::Id::new(("layer", track.id)),
                            egui::Sense::click(),
                        )
                        .clicked()
                    {
                        self.track = index;
                        self.selected = None;
                    }
                    painter.line_segment(
                        [row.left_bottom(), row.right_bottom()],
                        egui::Stroke::new(
                            1.0_f32,
                            ui.visuals().widgets.noninteractive.bg_stroke.color,
                        ),
                    );
                    for clip in &track.clips {
                        let rect = egui::Rect::from_min_max(
                            egui::pos2(x_for(clip.timeline_start), top + 4.0),
                            egui::pos2(
                                x_for(clip.end()).max(x_for(clip.timeline_start) + 4.0),
                                top + ROW_HEIGHT - 4.0,
                            ),
                        );
                        if !viewport.intersects(rect) {
                            continue;
                        }
                        let asset = clip
                            .source
                            .asset_id()
                            .and_then(|id| self.project.asset(id).ok());
                        let kind = asset.map(|a| a.kind).unwrap_or(MediaKind::Audio);
                        let voice = matches!(clip.source, ClipSource::Voicevox { .. });
                        let color = if matches!(clip.source, ClipSource::Text { .. }) {
                            egui::Color32::from_rgb(137, 80, 64)
                        } else if voice {
                            egui::Color32::from_rgb(117, 80, 150)
                        } else {
                            match kind {
                                MediaKind::Video => egui::Color32::from_rgb(47, 98, 143),
                                MediaKind::Image => egui::Color32::from_rgb(47, 122, 100),
                                MediaKind::Audio => egui::Color32::from_rgb(143, 100, 44),
                            }
                        };
                        painter.rect_filled(rect, 4.0, color);
                        if self.selected == Some(clip.id) {
                            painter.rect_stroke(
                                rect,
                                4.0,
                                egui::Stroke::new(2.0_f32, ui.visuals().selection.stroke.color),
                                egui::StrokeKind::Inside,
                            );
                        }
                        let title = match &clip.source {
                            ClipSource::Voicevox { text, subtitle, .. } => format!(
                                "{} · {text}",
                                self.tr(if subtitle.is_some() {
                                    "voice-with-subtitle"
                                } else {
                                    "voice-only"
                                })
                            ),
                            ClipSource::Text { settings } => format!(
                                "{} · {}",
                                self.tr(if settings.subtitle {
                                    "subtitle"
                                } else {
                                    "text-clip"
                                }),
                                settings.text
                            ),
                            _ => asset.map(|a| a.name.clone()).unwrap_or_default(),
                        };
                        let asset_id = clip.source.asset_id().unwrap_or(clip.id);
                        if clip.source.asset_id().is_some() {
                            decorate.push(asset_id);
                        }
                        let mut text_left = 5.0;
                        let mut text_y = rect.center().y;
                        match self.decorations.get(&asset_id) {
                            Some(crate::app::ClipDecoration::Image(texture))
                                if rect.width() > 90.0 =>
                            {
                                let thumb = egui::Rect::from_min_size(
                                    rect.min + egui::vec2(3.0, 3.0),
                                    egui::vec2(58.0, rect.height() - 6.0),
                                );
                                painter.with_clip_rect(rect.intersect(viewport)).image(
                                    texture.id(),
                                    thumb,
                                    egui::Rect::from_min_max(
                                        egui::Pos2::ZERO,
                                        egui::pos2(1.0, 1.0),
                                    ),
                                    egui::Color32::WHITE,
                                );
                                text_left = 65.0;
                            }
                            Some(crate::app::ClipDecoration::Waveform(wave)) => {
                                text_y = rect.top() + 10.0;
                                let max_duration = asset
                                    .map(|a| self.project.frame_rate.frames(a.duration_seconds))
                                    .unwrap_or(1)
                                    .max(1);
                                let columns = rect.width().ceil().clamp(1.0, 1024.0) as usize;
                                for bin in 0..columns {
                                    let fraction = bin as f32 / columns as f32;
                                    let source_frame =
                                        clip.source_offset as f32 + fraction * clip.duration as f32;
                                    let index = ((source_frame / max_duration as f32
                                        * wave.peaks.len() as f32)
                                        as usize)
                                        .min(wave.peaks.len() - 1);
                                    let end = (((clip.source_offset as f32
                                        + (bin + 1) as f32 / columns as f32 * clip.duration as f32)
                                        / max_duration as f32
                                        * wave.peaks.len() as f32)
                                        .ceil()
                                        as usize)
                                        .max(index + 1)
                                        .min(wave.peaks.len());
                                    let x = rect.left() + fraction * rect.width();
                                    let gain = clip.volume as f32;
                                    let half = wave.peaks[index..end]
                                        .iter()
                                        .copied()
                                        .fold(0.0_f32, f32::max)
                                        * gain;
                                    let rms =
                                        (wave.rms[index..end].iter().map(|v| v * v).sum::<f32>()
                                            / (end - index) as f32)
                                            .sqrt()
                                            * gain;
                                    let y = rect.bottom() - 10.0;
                                    painter.line_segment(
                                        [
                                            egui::pos2(x, y - half.min(1.0) * 8.0),
                                            egui::pos2(x, y + half.min(1.0) * 8.0),
                                        ],
                                        egui::Stroke::new(1.0_f32, egui::Color32::LIGHT_GRAY),
                                    );
                                    painter.line_segment(
                                        [
                                            egui::pos2(x, y - rms.min(1.0) * 8.0),
                                            egui::pos2(x, y + rms.min(1.0) * 8.0),
                                        ],
                                        egui::Stroke::new(1.0_f32, egui::Color32::WHITE),
                                    );
                                }
                            }
                            _ => {}
                        }
                        painter
                            .with_clip_rect(rect.shrink(3.0).intersect(viewport))
                            .text(
                                egui::pos2(rect.left() + text_left, text_y),
                                egui::Align2::LEFT_CENTER,
                                title,
                                egui::FontId::proportional(13.0),
                                egui::Color32::WHITE,
                            );
                        let hit = ui.interact(
                            rect,
                            egui::Id::new(("clip", clip.id)),
                            egui::Sense::click_and_drag(),
                        );
                        if hit.clicked() || hit.secondary_clicked() {
                            select = Some(clip.id);
                        }
                        hit.context_menu(|ui| {
                            if ui
                                .add_enabled(
                                    !track.locked
                                        && clip.timeline_start < self.playhead
                                        && self.playhead < clip.end(),
                                    egui::Button::new(self.tr("split")),
                                )
                                .clicked()
                            {
                                action = Some(ClipAction::Split(clip.id));
                                ui.close();
                            }
                            if ui
                                .add_enabled(!track.locked, egui::Button::new(self.tr("delete")))
                                .clicked()
                            {
                                action = Some(ClipAction::Delete(clip.id));
                                ui.close();
                            }
                            if ui
                                .add_enabled(
                                    !track.locked
                                        && track
                                            .clips
                                            .iter()
                                            .any(|c| c.timeline_start == clip.end()),
                                    egui::Button::new(self.tr("crossfade-next")),
                                )
                                .clicked()
                            {
                                action = Some(ClipAction::Crossfade(clip.id));
                                ui.close();
                            }
                            if voice && ui.button(self.tr("subtitle-from-voice")).clicked() {
                                // Applied after drawing through selection and a deferred flag.
                                select = Some(clip.id);
                                make_subtitle = true;
                                ui.close();
                            }
                        });
                        if hit.drag_started() && !track.locked {
                            select = Some(clip.id);
                            if let Some(pointer) = hit.interact_pointer_pos() {
                                let origin = pointer - hit.drag_delta();
                                self.drag = Some(DragClip {
                                    id: clip.id,
                                    offset: origin.x - rect.left(),
                                    start: clip.timeline_start,
                                    track: index,
                                });
                            }
                        }
                        if let Some(drag) = &mut self.drag
                            && drag.id == clip.id
                            && let Some(pointer) = hit.interact_pointer_pos()
                        {
                            let (target, start) = position(
                                egui::pos2(pointer.x - drag.offset, pointer.y),
                                canvas,
                                fps,
                                self.pixels_per_second,
                                self.project.tracks.len().saturating_sub(1),
                            );
                            let threshold = fps.frames(8.0 / self.pixels_per_second as f64);
                            let start = if self.snap {
                                commands::snap(
                                    &self.project,
                                    start,
                                    self.playhead,
                                    threshold,
                                    Some(clip.id),
                                )
                            } else {
                                start
                            };
                            drag.track = target;
                            drag.start = start;
                            if hit.drag_stopped() {
                                move_to = Some((drag.id, target, start));
                            }
                        }
                        hit.on_hover_text(format!(
                            "{} · {}",
                            self.tr("start-frame"),
                            clip.timeline_start
                        ));
                    }
                }
                if let Some(drag) = &self.drag {
                    let x = x_for(drag.start);
                    let y = canvas.top() + RULER_HEIGHT + drag.track as f32 * ROW_HEIGHT;
                    let valid = self.project.clip(drag.id).is_some_and(|c| {
                        self.project.tracks[drag.track].fits(drag.start, c.duration, Some(drag.id))
                            && !self.project.tracks[drag.track].locked
                    });
                    painter.rect_stroke(
                        egui::Rect::from_min_size(
                            egui::pos2(x, y + 2.0),
                            egui::vec2(30.0, ROW_HEIGHT - 4.0),
                        ),
                        2.0,
                        egui::Stroke::new(
                            2.0_f32,
                            if valid {
                                egui::Color32::LIGHT_GREEN
                            } else {
                                egui::Color32::LIGHT_RED
                            },
                        ),
                        egui::StrokeKind::Inside,
                    );
                }
                let head = x_for(self.playhead);
                painter.line_segment(
                    [
                        egui::pos2(head, canvas.top()),
                        egui::pos2(head, canvas.bottom()),
                    ],
                    egui::Stroke::new(2.0_f32, egui::Color32::from_rgb(230, 95, 80)),
                );
                let primary = egui::PointerButton::Primary;
                let seek_drag_id = response.id.with("seek-drag");
                if response.is_pointer_button_down_on() {
                    let from_timeline = ctx.input(|i| {
                        i.pointer
                            .press_origin()
                            .is_some_and(|point| point.x >= canvas.left() + LABEL_WIDTH)
                    });
                    ctx.data_mut(|data| data.insert_temp(seek_drag_id, from_timeline));
                }
                let seek_drag = (response.dragged_by(primary) || response.drag_stopped_by(primary))
                    && ctx.data(|data| data.get_temp::<bool>(seek_drag_id).unwrap_or(false));
                if self.drag.is_none()
                    && let Some(point) = response.interact_pointer_pos()
                    && (seek_drag
                        || response.clicked_by(primary) && point.x >= canvas.left() + LABEL_WIDTH)
                {
                    // A captured seek remains valid beyond the time origin.
                    // Clamp its final position too, even when motion and release
                    // arrive together; layer-label clicks do not start a seek.
                    let (_, frame) = position(
                        point,
                        canvas,
                        fps,
                        self.pixels_per_second,
                        self.project.tracks.len(),
                    );
                    self.seek_timeline(frame, ctx);
                }
                if response.drag_stopped() {
                    ctx.data_mut(|data| data.remove::<bool>(seek_drag_id));
                }
                let hovering = ctx.input(|i| !i.raw.hovered_files.is_empty());
                self.drop_target = None;
                if hovering
                    && let Some(point) = ctx.input(|i| i.pointer.latest_pos())
                    && viewport.contains(point)
                    && point.x >= canvas.left() + LABEL_WIDTH
                {
                    let (track, frame) = position(
                        point,
                        canvas,
                        fps,
                        self.pixels_per_second,
                        self.project.tracks.len(),
                    );
                    self.drop_target = Some((track, frame));
                    let top = canvas.top() + RULER_HEIGHT + track as f32 * ROW_HEIGHT;
                    painter.rect_stroke(
                        egui::Rect::from_min_size(
                            egui::pos2(x_for(frame), top),
                            egui::vec2(80.0, ROW_HEIGHT),
                        ),
                        2.0,
                        egui::Stroke::new(2.0_f32, egui::Color32::LIGHT_GREEN),
                        egui::StrokeKind::Inside,
                    );
                }
                let dropped = ctx.input(|i| i.raw.dropped_files.clone());
                if !dropped.is_empty() {
                    let paths: Vec<_> = dropped.into_iter().filter_map(|f| f.path).collect();
                    if !paths.is_empty() {
                        let point = ctx.input(|i| i.pointer.latest_pos());
                        if let Some(point) = point
                            .filter(|p| viewport.contains(*p) && p.x >= canvas.left() + LABEL_WIDTH)
                        {
                            let (track, frame) = position(
                                point,
                                canvas,
                                fps,
                                self.pixels_per_second,
                                self.project.tracks.len(),
                            );
                            self.import(paths, track, frame);
                        } else {
                            // Wayland compositors may omit drop coordinates. Never guess from a stale pointer.
                            self.status = self.tr("drop-fallback");
                            self.import(paths, self.track, self.playhead);
                        }
                    }
                }
            });
        if let Some(id) = select {
            self.select(id);
        }
        // Mutate only after drawing; the menu always targets the right-clicked clip.
        match action {
            Some(ClipAction::Split(id)) => {
                let frame = self.playhead;
                self.edit(|p| commands::split(p, id, frame));
            }
            Some(ClipAction::Delete(id)) => self.edit(|p| commands::delete(p, id)),
            Some(ClipAction::Crossfade(id)) => {
                let duration = self.project.frame_rate.frames(0.5);
                self.edit(|p| commands::crossfade_next(p, id, duration).map(|_| ()));
            }
            None => {}
        }
        if make_subtitle {
            self.insert_text(true, true);
        }
        for id in decorate {
            self.request_decoration(id);
        }
        if let Some((id, track, frame)) = move_to {
            self.edit(|p| commands::move_clip(p, id, track, frame));
            self.drag = None;
        }
        if self.drag.is_some() && !ctx.input(|i| i.pointer.any_down()) {
            self.drag = None;
        }
    }
}
