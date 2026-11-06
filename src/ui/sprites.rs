mod thumbnails;

use super::searchable_combo_box::{Choice, Labels, SearchState, searchable_combo_box};
use crate::{
    app::{App, PathMode},
    project::*,
    sprites::{
        psd::{Document, Node},
        *,
    },
    tasks::LatestWorker,
};
use eframe::egui;
use std::{path::PathBuf, sync::Arc};

struct PreviewRequest {
    sprite: Sprite,
    frame: u64,
    fps: FrameRate,
    speaking: bool,
}
struct SettingsPreview {
    document: Arc<Document>,
    image: crate::Result<image::RgbaImage>,
}
#[derive(Default)]
pub struct SpriteUi {
    pub open: bool,
    pub selected: Option<Id>,
    pub draft: Option<Sprite>,
    pub error: Option<String>,
    tab: u8,
    delete: Option<Id>,
    search: SearchState,
    clip_search: SearchState,
    character_search: SearchState,
    document: Option<Arc<Document>>,
    texture: Option<egui::TextureHandle>,
    preview_frame: u64,
    speaking: bool,
    worker: Option<LatestWorker<PreviewRequest, SettingsPreview>>,
    ticket: u64,
    last_preview: Option<Vec<u8>>,
    name_composing: bool,
    preview_ready: bool,
    thumbnails: thumbnails::LayerThumbnails,
}
impl App {
    pub(super) fn warn_missing_sprites(&mut self) {
        for clip in self.project.tracks.iter().flat_map(|t| &t.clips) {
            if let ClipSource::Sprite { settings_id } = clip.source {
                match self.sprites.library.get(settings_id) {
                    None => {
                        self.status = format!("{} [{settings_id}]", self.tr("sprite-missing"));
                        return;
                    }
                    Some(sprite) if !sprite.psd.is_file() => {
                        self.status = format!(
                            "{}: {}",
                            self.tr("sprite-psd-missing"),
                            sprite.psd.display()
                        );
                        return;
                    }
                    _ => {}
                }
            }
        }
    }

    pub fn save_sprites(&mut self, library: SpriteLibrary) -> crate::Result<()> {
        self.sprites.save(library)?;
        self.changed();
        Ok(())
    }
    pub fn reload_sprites(&mut self) {
        let mut store = SpriteStore::load(&self.storage.config);
        if let Some(error) = &store.error {
            self.status = self.local_error(error);
            store.library = self.sprites.library.clone();
        }
        self.sprites = store;
        self.sprite_ui.draft = None;
        self.sprite_ui.last_preview = None;
        self.changed();
    }
    pub fn set_sprite_psd(&mut self, id: Id, path: PathBuf) {
        if let Err(error) = crate::assets::require_utf8_path(&path) {
            self.sprite_ui.error = Some(error);
            return;
        }
        let Some(mut sprite) = self
            .sprite_ui
            .draft
            .as_ref()
            .filter(|s| s.id == id)
            .cloned()
            .or_else(|| self.sprites.library.get(id).cloned())
        else {
            return;
        };
        if sprite.psd != path {
            sprite.psd = path;
            sprite.visibility.clear();
            sprite.mouth = Animation::default();
            sprite.blink = Animation::default();
        }
        // Parse and validate asynchronously in the settings preview worker.
        let mut library = self.sprites.library.clone();
        if let Some(old) = library.sprites.iter_mut().find(|s| s.id == id) {
            *old = sprite.clone();
        }
        match self.save_sprites(library) {
            Ok(()) => self.sprite_ui.error = None,
            Err(e) => self.sprite_ui.error = Some(e),
        }
        self.sprite_ui.draft = Some(sprite);
        // A result already queued for the previous PSD must not restore its
        // document/face preview before the replacement request is submitted.
        if let Some(worker) = &self.sprite_ui.worker {
            worker.cancel();
        }
        self.sprite_ui.ticket = 0;
        self.sprite_ui.document = None;
        self.sprite_ui.texture = None;
        self.sprite_ui.preview_ready = false;
        self.sprite_ui.last_preview = None;
    }
    pub fn insert_sprite(&mut self, settings_id: Id) {
        let Some(sprite) = self.sprites.library.get(settings_id) else {
            self.status = self.tr("sprite-missing");
            return;
        };
        if !sprite.psd.is_file() {
            self.status = self.tr("sprite-psd-missing");
            return;
        }
        let clip = Clip {
            id: Id::new_v4(),
            character: None,
            source: ClipSource::Sprite { settings_id },
            timeline_start: self.playhead,
            duration: self.project.frame_rate.frames(5.0),
            source_offset: 0,
            volume: 1.0,
            muted: false,
            effects: ClipEffects::default(),
        };
        let id = clip.id;
        let preferred = self.track;
        self.finish_preview_transform();
        let result = self.history.edit(&mut self.project, |p| {
            let target =
                commands::placement_track(p, preferred, clip.timeline_start, clip.duration)?;
            commands::insert(p, target, clip)?;
            Ok(target)
        });
        if let Some(target) = self.report(result) {
            self.changed();
            self.focus_clip(id);
            if target != preferred {
                self.status = self.tr("inserted-safe-layer");
            }
        }
    }
    pub(super) fn sprite_clip_properties(&mut self, ui: &mut egui::Ui, settings_id: &mut Id) {
        ui.strong(self.tr("sprite"));
        let mut search = std::mem::take(&mut self.sprite_ui.clip_search);
        self.sprite_picker(ui, settings_id, &mut search, "clip-sprite");
        self.sprite_ui.clip_search = search;
        if let Some(sprite) = self.sprites.library.get(*settings_id) {
            if !sprite.psd.is_file() {
                ui.colored_label(ui.visuals().error_fg_color, self.tr("sprite-psd-missing"));
            }
            if sprite
                .character_id
                .is_some_and(|id| self.characters.library.get(id).is_none())
            {
                ui.small(self.tr("character-missing"));
            }
        } else {
            ui.colored_label(ui.visuals().error_fg_color, self.tr("sprite-missing"));
        }
        if ui.button(self.tr("sprite-settings")).clicked() {
            self.sprite_ui.open = true;
            self.sprite_ui.selected = Some(*settings_id);
            self.sprite_ui.draft = None;
        }
    }
    fn sprite_picker(
        &self,
        ui: &mut egui::Ui,
        id: &mut Id,
        search: &mut SearchState,
        salt: impl std::hash::Hash,
    ) -> bool {
        let choices: Vec<_> = self
            .sprites
            .library
            .sprites
            .iter()
            .map(|s| Choice::new(s.id, format!("{} [{}]", s.name, &s.id.to_string()[..8])))
            .collect();
        searchable_combo_box(
            ui,
            salt,
            search,
            id,
            &choices,
            Labels {
                search: &self.tr("combo-search"),
                no_results: &self.tr("combo-no-results"),
                fallback: &self.tr("sprite-select"),
            },
        )
        .changed()
    }
    pub(super) fn sprite_window(&mut self, ctx: &egui::Context) {
        if !self.sprite_ui.open {
            return;
        }
        let mut state = std::mem::take(&mut self.sprite_ui);
        if let Some(worker) = &state.worker {
            while let Ok((ticket, result)) = worker.receiver.try_recv() {
                if ticket != state.ticket {
                    continue;
                }
                match result {
                    Ok(preview) => {
                        state.document = Some(preview.document);
                        match preview.image {
                            Ok(image) => {
                                state.preview_ready = true;
                                state.texture = Some(ctx.load_texture(
                                    "sprite-settings-preview",
                                    egui::ColorImage::from_rgba_unmultiplied(
                                        [image.width() as usize, image.height() as usize],
                                        image.as_raw(),
                                    ),
                                    egui::TextureOptions::LINEAR,
                                ));
                                if self.sprites.library.get(state.selected.unwrap_or_default())
                                    == state.draft.as_ref()
                                {
                                    state.error = None;
                                }
                            }
                            Err(error) => {
                                state.preview_ready = false;
                                state.texture = None;
                                state.error = Some(error);
                            }
                        }
                    }
                    Err(error) => {
                        state.preview_ready = false;
                        state.document = None;
                        state.texture = None;
                        state.error = Some(error);
                    }
                }
            }
        }
        state.thumbnails.begin_frame(state.document.as_ref(), ctx);
        let mut open = state.open;
        let mut choose_psd = None;
        let mut insert = None;
        egui::Window::new(self.tr("sprite-settings"))
            .id(egui::Id::new("sprite-manager"))
            .open(&mut open)
            .default_size([760.0, 650.0])
            .show(ctx, |ui| {
                if let Some(error) = self.sprites.error.as_ref().or(state.error.as_ref()) {
                    ui.colored_label(ui.visuals().error_fg_color, self.local_error(error));
                }
                ui.horizontal_wrapped(|ui| {
                    let mut selected = state.selected.unwrap_or_default();
                    if self.sprite_picker(ui, &mut selected, &mut state.search, "manager-sprite") {
                        state.selected = Some(selected);
                        state.draft = None;
                        state.name_composing = false;
                        state.document = None;
                        state.texture = None;
                        state.last_preview = None;
                    }
                    if ui.button(self.tr("sprite-new")).clicked() {
                        let sprite = Sprite::new(self.tr("sprite-new-name"));
                        let id = sprite.id;
                        let mut library = self.sprites.library.clone();
                        library.sprites.push(sprite);
                        match self.save_sprites(library) {
                            Ok(()) => {
                                state.selected = Some(id);
                                state.draft = None;
                                state.name_composing = false;
                                state.last_preview = None;
                                state.error = None;
                            }
                            Err(e) => state.error = Some(e),
                        }
                    }
                    if ui
                        .add_enabled(
                            state.selected.is_some(),
                            egui::Button::new(self.tr("character-duplicate")),
                        )
                        .clicked()
                        && let Some(mut sprite) = state
                            .selected
                            .and_then(|id| self.sprites.library.get(id))
                            .cloned()
                    {
                        sprite.id = Id::new_v4();
                        sprite.name = format!("{} ({})", sprite.name, self.tr("character-copy"));
                        let id = sprite.id;
                        let mut library = self.sprites.library.clone();
                        library.sprites.push(sprite);
                        match self.save_sprites(library) {
                            Ok(()) => {
                                state.selected = Some(id);
                                state.draft = None;
                                state.last_preview = None;
                                state.error = None;
                            }
                            Err(e) => state.error = Some(e),
                        }
                    }
                    if ui
                        .add_enabled(
                            state.selected.is_some(),
                            egui::Button::new(self.tr("delete")),
                        )
                        .clicked()
                    {
                        state.delete = state.selected;
                    }
                    if ui.button(self.tr("characters-reload")).clicked() {
                        self.reload_sprites();
                        state.draft = None;
                        state.last_preview = None;
                        state.error = None;
                    }
                });
                ui.small(self.tr("sprite-shared-hint"));
                if state.draft.as_ref().map(|s| s.id) != state.selected {
                    state.preview_ready = false;
                    state.draft = state
                        .selected
                        .and_then(|id| self.sprites.library.get(id))
                        .cloned();
                }
                let Some(draft) = &mut state.draft else {
                    return;
                };
                let before = draft.clone();
                let mut name_preedit = false;
                let mut name_commit = false;
                ui.horizontal(|ui| {
                    ui.label(self.tr("character-name"));
                    let response = ui.add(
                        egui::TextEdit::singleline(&mut draft.name)
                            .id(egui::Id::new(("sprite-name", draft.id)))
                            .return_key(None)
                            .char_limit(256),
                    );
                    if response.has_focus() || response.lost_focus() {
                        ui.input(|i| {
                            for e in &i.events {
                                if let egui::Event::Ime(e) = e {
                                    match e {
                                        egui::ImeEvent::Preedit(t) => {
                                            state.name_composing = !t.is_empty();
                                            name_preedit = true;
                                        }
                                        egui::ImeEvent::Commit(_) => {
                                            state.name_composing = false;
                                            name_commit = true;
                                        }
                                        egui::ImeEvent::Disabled => state.name_composing = false,
                                        _ => {}
                                    }
                                }
                            }
                        });
                    }
                    if !response.has_focus() {
                        state.name_composing = false;
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    for (tab, key) in [
                        (0, "sprite-basic"),
                        (1, "sprite-mouth"),
                        (2, "sprite-blink"),
                    ] {
                        ui.selectable_value(&mut state.tab, tab, self.tr(key));
                    }
                });
                egui::ScrollArea::vertical()
                    .id_salt("sprite-editor-scroll")
                    .max_height(350.0)
                    .show(ui, |ui| {
                        ui.push_id(("sprite-editor", draft.id, state.tab), |ui| {
                            match state.tab {
                                0 => {
                                    if ui.button(self.tr("sprite-choose-psd")).clicked() {
                                        choose_psd = Some(draft.id);
                                    }
                                    ui.label(draft.psd.to_string_lossy());
                                    ui.label(self.tr("character"));
                                    self.character_picker(
                                        ui,
                                        &mut draft.character_id,
                                        &mut state.character_search,
                                        ("sprite-character", draft.id),
                                    );
                                    if ui.small_button(self.tr("sprite-no-character")).clicked() {
                                        draft.character_id = None;
                                    }
                                    if let Some(doc) = &state.document {
                                        ui.label(self.tr("sprite-layers"));
                                        layer_tree(ui, &doc.nodes, &mut draft.visibility);
                                        if ui.button(self.tr("sprite-reset-layers")).clicked() {
                                            draft.visibility.clear();
                                        }
                                    }
                                }
                                1 | 2 => {
                                    let mouth = state.tab == 1;
                                    let animation = if mouth {
                                        &mut draft.mouth
                                    } else {
                                        &mut draft.blink
                                    };
                                    animation_editor(
                                        ui,
                                        animation,
                                        state.document.as_ref(),
                                        &self.i18n,
                                        mouth,
                                        &mut state.thumbnails,
                                    );
                                    if !mouth {
                                        ui.horizontal(|ui| {
                                            ui.label(self.tr("sprite-blink-interval"));
                                            ui.add(
                                                egui::DragValue::new(&mut draft.blink_interval_ms)
                                                    .range(100..=600000)
                                                    .suffix(" ms"),
                                            );
                                        });
                                    }
                                }
                                _ => {}
                            }
                        });
                    });
                let ready = !state.name_composing
                    && (!name_preedit || name_commit)
                    && !draft.name.trim().is_empty();
                let save = ui
                    .add_enabled(ready, egui::Button::new(self.tr("characters-save")))
                    .clicked();
                if ready && (*draft != before || name_commit || save) {
                    let mut library = self.sprites.library.clone();
                    if let Some(old) = library.sprites.iter_mut().find(|s| s.id == draft.id) {
                        *old = draft.clone();
                    }
                    match self.save_sprites(library) {
                        Ok(()) => state.error = None,
                        Err(e) => state.error = Some(e),
                    }
                }
                ui.separator();
                ui.label(self.tr("sprite-animation-preview"));
                ui.horizontal(|ui| {
                    ui.checkbox(&mut state.speaking, self.tr("sprite-preview-speaking"));
                    ui.add(
                        egui::Slider::new(&mut state.preview_frame, 0..=600)
                            .text(self.tr("playhead")),
                    );
                });
                if let Some(texture) = &state.texture {
                    let size = texture.size_vec2();
                    ui.add(
                        egui::Image::new(texture)
                            .fit_to_exact_size(size * (180.0 / size.y).min(1.0)),
                    );
                }
                if ui
                    .add_enabled(
                        state.preview_ready
                            && *draft == before
                            && state.document.is_some()
                            && self.sprites.library.get(draft.id) == Some(draft),
                        egui::Button::new(self.tr("sprite-insert")),
                    )
                    .clicked()
                {
                    insert = Some(draft.id);
                }
            });
        if let Some(id) = state.delete {
            egui::Window::new(self.tr("sprite-delete-title"))
                .id(egui::Id::new("sprite-delete-confirm"))
                .collapsible(false)
                .show(ctx, |ui| {
                    ui.label(self.tr("sprite-delete-warning"));
                    if let Some(sprite) = self.sprites.library.get(id) {
                        ui.label(format!("{} [{id}]", sprite.name));
                    }
                    ui.horizontal(|ui| {
                        if ui.button(self.tr("delete")).clicked() {
                            let mut library = self.sprites.library.clone();
                            library.sprites.retain(|s| s.id != id);
                            match self.save_sprites(library) {
                                Ok(()) => {
                                    state.selected = None;
                                    state.draft = None;
                                    state.document = None;
                                    state.texture = None;
                                    state.delete = None;
                                    state.error = None;
                                }
                                Err(e) => state.error = Some(e),
                            }
                        }
                        if ui.button(self.tr("cancel")).clicked() {
                            state.delete = None;
                        }
                    });
                });
        }
        // Pure timeline-frame evaluation; dragging this slider previews both animations.
        if let Some(draft) = &state.draft {
            let key = serde_json::to_vec(&(
                draft,
                state.preview_frame,
                state.speaking,
                self.project.frame_rate,
            ))
            .ok();
            if state.last_preview != key
                && !draft.psd.as_os_str().is_empty()
                && !state.name_composing
            {
                if state.worker.is_none() {
                    state.worker = Some(LatestWorker::with_processor(
                        ctx.clone(),
                        |request: PreviewRequest, cancelled| {
                            if cancelled() {
                                return Err("cancelled".into());
                            }
                            let doc = psd::load(&request.sprite.psd)?;
                            let image = (|| {
                                doc.validate_settings(&request.sprite)?;
                                let render = SpriteRender {
                                    source_revision: None,
                                    settings: request.sprite,
                                    speech: if request.speaking {
                                        vec![SpeechSpan {
                                            start: 0,
                                            end: MAX_FRAME,
                                        }]
                                    } else {
                                        Vec::new()
                                    },
                                };
                                let visible = doc.visibility(
                                    &render,
                                    request.frame,
                                    request.frame,
                                    request.fps,
                                )?;
                                let image = doc.composite_cancellable(&visible, cancelled)?;
                                Ok(image::DynamicImage::ImageRgba8(image)
                                    .thumbnail(512, 512)
                                    .to_rgba8())
                            })();
                            Ok(SettingsPreview {
                                document: doc,
                                image,
                            })
                        },
                    ));
                }
                state.ticket = state
                    .worker
                    .as_ref()
                    .expect("initialized sprite preview worker")
                    .request(PreviewRequest {
                        sprite: draft.clone(),
                        frame: state.preview_frame,
                        fps: self.project.frame_rate,
                        speaking: state.speaking,
                    });
                state.preview_ready = false;
                state.last_preview = key;
            }
        }
        state.thumbnails.finish_frame(ctx);
        state.open = open;
        self.sprite_ui = state;
        if let Some(id) = choose_psd {
            self.request_path(PathMode::SpritePsd(id));
        }
        if let Some(id) = insert {
            self.insert_sprite(id);
        }
    }
}
fn layer_tree(
    ui: &mut egui::Ui,
    nodes: &[Node],
    visibility: &mut std::collections::BTreeMap<String, bool>,
) {
    for node in nodes.iter().rev() {
        ui.push_id(&node.key, |ui| {
            let mut visible = visibility.get(&node.key).copied().unwrap_or(node.visible);
            if ui
                .checkbox(&mut visible, &node.name)
                .on_hover_text(format!(
                    "{} · {},{} · {}%",
                    node.key,
                    node.rect[0],
                    node.rect[1],
                    node.opacity as u32 * 100 / 255
                ))
                .changed()
            {
                visibility.insert(node.key.clone(), visible);
            }
            if node.group {
                ui.indent("children", |ui| layer_tree(ui, &node.children, visibility));
            }
        });
    }
}
fn layer_selector(
    ui: &mut egui::Ui,
    keys: &mut Vec<String>,
    doc: Option<&Arc<Document>>,
    salt: impl std::hash::Hash,
    label: String,
    thumbnails: &mut thumbnails::LayerThumbnails,
    i18n: &crate::i18n::I18n,
) {
    egui::ComboBox::from_id_salt(salt)
        .selected_text(format!("{label} ({})", keys.len()))
        .height(220.0)
        .show_ui(ui, |ui| {
            if let Some(doc) = doc {
                ui.set_min_width(250.0);
                let nodes: Vec<_> = doc.all_nodes().into_iter().rev().collect();
                // Virtualize long PSD lists; request images only for visible rows.
                egui::ScrollArea::vertical().max_height(180.0).show_rows(
                    ui,
                    40.0,
                    nodes.len(),
                    |ui, rows| {
                        for row in rows {
                            let node = nodes[row];
                            ui.push_id(&node.key, |ui| {
                                ui.horizontal(|ui| {
                                    let mut selected = keys.contains(&node.key);
                                    let clicked = thumbnails.show(ui, node, i18n).clicked();
                                    let changed = ui
                                        .checkbox(
                                            &mut selected,
                                            format!("{} [{}]", node.name, node.index),
                                        )
                                        .changed();
                                    if clicked {
                                        selected = !selected;
                                    }
                                    if changed || clicked {
                                        keys.retain(|key| key != &node.key);
                                        if selected {
                                            keys.push(node.key.clone());
                                        }
                                    }
                                });
                            });
                        }
                    },
                );
            }
        })
        .response
        .on_hover_text(i18n.text("sprite-animation-layer-hint"));
}
fn animation_editor(
    ui: &mut egui::Ui,
    animation: &mut Animation,
    doc: Option<&Arc<Document>>,
    i18n: &crate::i18n::I18n,
    mouth: bool,
    thumbnails: &mut thumbnails::LayerThumbnails,
) {
    ui.checkbox(&mut animation.enabled, i18n.text("enabled"));
    ui.label(i18n.text("sprite-idle"));
    layer_selector(
        ui,
        &mut animation.idle_layers,
        doc,
        "idle",
        i18n.text("sprite-select-layers"),
        thumbnails,
        i18n,
    );
    let mut speed = animation.speed_milli as f64 / 1000.0;
    if ui
        .add(egui::Slider::new(&mut speed, 0.1..=10.0).text(i18n.text("sprite-speed")))
        .changed()
    {
        animation.speed_milli = (speed * 1000.0).round() as u32;
    }
    if mouth {
        ui.checkbox(&mut animation.repeat, i18n.text("sprite-repeat"));
    }
    let mut action = None;
    let len = animation.frames.len();
    for (i, frame) in animation.frames.iter_mut().enumerate() {
        ui.push_id(i, |ui| {
            ui.horizontal(|ui| {
                ui.label(format!("{} {}", i18n.text("sprite-frame"), i + 1));
                ui.add(
                    egui::DragValue::new(&mut frame.duration_ms)
                        .range(1..=60000)
                        .suffix(" ms"),
                );
                if ui.add_enabled(i > 0, egui::Button::new("↑")).clicked() {
                    action = Some((i, 0));
                }
                if ui
                    .add_enabled(i + 1 < len, egui::Button::new("↓"))
                    .clicked()
                {
                    action = Some((i, 1));
                }
                if ui
                    .add_enabled(len > 1, egui::Button::new(i18n.text("delete")))
                    .clicked()
                {
                    action = Some((i, 2));
                }
            });
            layer_selector(
                ui,
                &mut frame.layers,
                doc,
                "layers",
                i18n.text("sprite-select-layers"),
                thumbnails,
                i18n,
            );
        });
    }
    if let Some((i, kind)) = action {
        match kind {
            0 => animation.frames.swap(i, i - 1),
            1 => animation.frames.swap(i, i + 1),
            _ => {
                animation.frames.remove(i);
            }
        }
    }
    if ui
        .add_enabled(len < 128, egui::Button::new(i18n.text("sprite-add-frame")))
        .clicked()
    {
        animation.frames.push(AnimationFrame::default());
    }
}
