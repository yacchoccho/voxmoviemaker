//! Theme-aware, virtualized selection UI. Searching never changes the selected value.
use eframe::egui::{self, Key, Modifiers};

#[derive(Clone, Debug)]
pub struct Choice<T> {
    pub value: T,
    label: String,
    search_label: String,
}
impl<T> Choice<T> {
    pub fn new(value: T, label: impl Into<String>) -> Self {
        let label = label.into();
        let search_label = label.to_lowercase();
        Self {
            value,
            label,
            search_label,
        }
    }
    pub fn set_label(&mut self, label: String) {
        if self.label != label {
            self.search_label = label.to_lowercase();
            self.label = label;
        }
    }
    pub fn label(&self) -> &str {
        &self.label
    }
}

pub struct Labels<'a> {
    pub search: &'a str,
    pub no_results: &'a str,
    pub fallback: &'a str,
}

#[derive(Debug, Default)]
pub struct SearchState {
    owner: Option<egui::Id>,
    open: bool,
    query: String,
    highlighted: Option<usize>,
    composing: bool,
    scroll_offset: f32,
    viewport_height: f32,
}
impl SearchState {
    pub fn is_open(&self) -> bool {
        self.open
    }
}

/// Values and display labels are independent; neither filtering nor opening commits a value.
/// Keep one state per control and use a stable salt that includes the edited object's ID.
pub fn searchable_combo_box<T: Clone + PartialEq>(
    ui: &mut egui::Ui,
    salt: impl std::hash::Hash,
    state: &mut SearchState,
    selected: &mut T,
    choices: &[Choice<T>],
    labels: Labels<'_>,
) -> egui::Response {
    let id = ui.make_persistent_id(salt);
    if state.owner != Some(id) {
        *state = SearchState {
            owner: Some(id),
            ..Default::default()
        };
    }
    if !ui.is_enabled() {
        state.open = false;
    }
    let current = choices.iter().position(|c| c.value == *selected);
    let label = current.map_or(labels.fallback, |i| choices[i].label.as_str());
    let mut button = ui
        .push_id(id, |ui| {
            ui.add_sized(
                [ui.available_width(), ui.spacing().interact_size.y],
                egui::Button::new(format!("{label}  ▾")).truncate(),
            )
        })
        .inner;
    let mut opening = false;
    if button.clicked() {
        state.open = !state.open;
        if state.open {
            state.query.clear();
            state.composing = false;
            state.highlighted = current;
            state.scroll_offset = 0.0;
            opening = true;
        }
    }
    let search_id = id.with("search");
    let mut open = state.open;
    let mut accepted = None;
    let mut dismissed = false;
    egui::Popup::from_response(&button)
        .id(id.with("popup"))
        .open_bool(&mut open)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .width(button.rect.width())
        .show(|ui| {
            ui.set_width(button.rect.width().max(100.0) - 16.0);
            dismissed = ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Escape));
            // Consume navigation before TextEdit handles it, but leave IME keys alone.
            // A Commit (or final empty Preedit) may share a frame with Enter.
            let was_composing = state.composing;
            let mut ime_event = false;
            ui.input(|input| {
                for event in &input.events {
                    if let egui::Event::Ime(event) = event {
                        match event {
                            egui::ImeEvent::Preedit(text) => {
                                state.composing = !text.is_empty();
                                ime_event = true;
                            }
                            egui::ImeEvent::Commit(_) => {
                                state.composing = false;
                                ime_event = true;
                            }
                            egui::ImeEvent::Disabled => state.composing = false,
                            egui::ImeEvent::Enabled => {}
                        }
                    }
                }
            });
            let safe_keys = !was_composing && !state.composing && !ime_event;
            let (up, down, enter) = ui.input_mut(|input| {
                if safe_keys {
                    (
                        input.consume_key(Modifiers::NONE, Key::ArrowUp),
                        input.consume_key(Modifiers::NONE, Key::ArrowDown),
                        input.consume_key(Modifiers::NONE, Key::Enter),
                    )
                } else {
                    (false, false, false)
                }
            });
            let search = ui.add(
                egui::TextEdit::singleline(&mut state.query)
                    .id(search_id)
                    .return_key(None)
                    .hint_text(labels.search)
                    .desired_width(f32::INFINITY),
            );
            if opening {
                search.request_focus();
            }
            let filtered = matching_indices(choices, &state.query);
            if search.changed() || state.highlighted.is_some_and(|i| !filtered.contains(&i)) {
                state.highlighted = filtered.first().copied();
            }
            let mut reveal = opening || search.changed();
            if up || down {
                let position = state
                    .highlighted
                    .and_then(|i| filtered.iter().position(|&n| n == i));
                let next = match (position, down) {
                    (Some(i), true) => (i + 1).min(filtered.len().saturating_sub(1)),
                    (Some(i), false) => i.saturating_sub(1),
                    (None, true) => 0,
                    (None, false) => filtered.len().saturating_sub(1),
                };
                state.highlighted = filtered.get(next).copied();
                reveal = true;
            }
            if enter {
                accepted = state.highlighted.filter(|i| filtered.contains(i));
            }
            if filtered.is_empty() {
                ui.label(labels.no_results);
                state.scroll_offset = 0.0;
                return;
            }
            let row_height = ui.spacing().interact_size.y.max(
                ui.text_style_height(&egui::TextStyle::Button)
                    + ui.spacing().button_padding.y * 2.0,
            );
            let stride = row_height + ui.spacing().item_spacing.y;
            let max_height = 240.0_f32.min((ui.ctx().content_rect().height() - 100.0).max(40.0));
            let mut scroll = egui::ScrollArea::vertical()
                .id_salt(id.with("list"))
                .max_height(max_height)
                .animated(false);
            if reveal {
                let index = state
                    .highlighted
                    .and_then(|i| filtered.iter().position(|&n| n == i))
                    .unwrap_or(0);
                let top = index as f32 * stride;
                let height = if state.viewport_height > 0.0 {
                    state.viewport_height.min(max_height)
                } else {
                    max_height
                };
                let offset = state
                    .scroll_offset
                    .min(top)
                    .max(top + row_height - height)
                    .max(0.0);
                scroll = scroll.vertical_scroll_offset(offset);
            }
            let output = scroll.show_rows(ui, row_height, filtered.len(), |ui, range| {
                for row in range {
                    let i = filtered[row];
                    let label = if current == Some(i) {
                        format!("✓ {}", choices[i].label)
                    } else {
                        choices[i].label.clone()
                    };
                    let response = ui
                        .add_sized(
                            [ui.available_width(), row_height],
                            egui::Button::selectable(state.highlighted == Some(i), label)
                                .truncate(),
                        )
                        .on_hover_text(&choices[i].label);
                    if response.clicked() {
                        accepted = Some(i);
                    }
                }
            });
            state.scroll_offset = output.state.offset.y;
            state.viewport_height = output.inner_rect.height();
        });
    if let Some(i) = accepted.filter(|_| !dismissed) {
        if *selected != choices[i].value {
            *selected = choices[i].value.clone();
            button.mark_changed();
        }
        open = false;
    }
    if dismissed {
        open = false;
    }
    if state.open && !open {
        ui.memory_mut(|m| m.surrender_focus(search_id));
        state.composing = false;
    }
    state.open = open;
    button
}

fn matching_indices<T>(choices: &[Choice<T>], query: &str) -> Vec<usize> {
    let query = query.to_lowercase();
    choices
        .iter()
        .enumerate()
        .filter_map(|(i, c)| c.search_label.contains(&query).then_some(i))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filtering_is_case_insensitive_partial_and_supports_japanese() {
        let items = [
            Choice::new(1, "Noto Sans 日本語"),
            Choice::new(2, "DejaVu Serif"),
            Choice::new(3, "NOTO Serif"),
        ];
        assert_eq!(matching_indices(&items, ""), [0, 1, 2]);
        assert_eq!(matching_indices(&items, "nOtO"), [0, 2]);
        assert_eq!(matching_indices(&items, "sans 日本"), [0]);
        assert!(matching_indices(&items, "absent").is_empty());
    }
}
