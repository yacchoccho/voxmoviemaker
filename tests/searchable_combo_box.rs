use eframe::egui::{self, Event, Key, Modifiers, PointerButton};
use std::sync::Arc;
use voxmoviemaker::ui::searchable_combo_box::{Choice, Labels, SearchState, searchable_combo_box};

struct Fixture {
    ctx: egui::Context,
    state: SearchState,
    selected: usize,
    choices: Vec<Choice<usize>>,
    button: egui::Rect,
}
impl Fixture {
    fn new(choices: Vec<Choice<usize>>, selected: usize) -> Self {
        let ctx = egui::Context::default();
        let mut fonts = egui::FontDefinitions::empty();
        let bytes = std::fs::read("/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc")
            .or_else(|_| std::fs::read("/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf"))
            .expect("a system font is required for UI tests");
        fonts
            .font_data
            .insert("system".into(), Arc::new(egui::FontData::from_owned(bytes)));
        fonts
            .families
            .insert(egui::FontFamily::Proportional, vec!["system".into()]);
        ctx.set_fonts(fonts);
        Self {
            ctx,
            state: Default::default(),
            selected,
            choices,
            button: egui::Rect::NOTHING,
        }
    }
    fn frame(&mut self, events: Vec<Event>) -> egui::FullOutput {
        self.ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(600.0, 500.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.set_max_width(220.0);
                    self.button = searchable_combo_box(
                        ui,
                        "test-picker",
                        &mut self.state,
                        &mut self.selected,
                        &self.choices,
                        Labels {
                            search: "Search…",
                            no_results: "検索結果がありません",
                            fallback: "Unlisted value",
                        },
                    )
                    .rect;
                });
            },
        )
    }
    fn click(&mut self, pos: egui::Pos2) {
        self.frame(vec![Event::PointerMoved(pos)]);
        for pressed in [true, false] {
            self.frame(vec![Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            }]);
        }
    }
    fn open(&mut self) -> egui::FullOutput {
        self.frame(vec![]);
        self.click(self.button.center());
        self.frame(vec![]);
        assert!(self.state.is_open());
        self.frame(vec![])
    }
    fn key(&mut self, key: Key) -> egui::FullOutput {
        self.frame(vec![key_event(key, false, Modifiers::NONE)]);
        self.frame(vec![key_event(key, true, Modifiers::NONE)])
    }
    fn query(&mut self, text: &str) -> egui::FullOutput {
        let select_all = Modifiers::CTRL | Modifiers::COMMAND;
        self.frame(vec![key_event(Key::A, true, select_all)]);
        self.frame(vec![key_event(Key::A, false, select_all)]);
        if text.is_empty() {
            self.key(Key::Backspace);
        } else {
            self.frame(vec![Event::Text(text.into())]);
        }
        self.frame(vec![])
    }
}
fn key_event(key: Key, pressed: bool, modifiers: Modifiers) -> Event {
    Event::Key {
        key,
        pressed,
        physical_key: None,
        repeat: false,
        modifiers,
    }
}
fn position(shape: &egui::epaint::Shape, label: &str) -> Option<egui::Pos2> {
    match shape {
        egui::epaint::Shape::Text(text) if text.galley.text() == label => {
            Some(text.pos + text.galley.size() / 2.0)
        }
        egui::epaint::Shape::Vec(shapes) => shapes.iter().find_map(|s| position(s, label)),
        _ => None,
    }
}
fn find(output: &egui::FullOutput, label: &str) -> Option<egui::Pos2> {
    output.shapes.iter().find_map(|s| position(&s.shape, label))
}
fn texts(output: &egui::FullOutput) -> Vec<String> {
    output
        .shapes
        .iter()
        .filter_map(|s| match &s.shape {
            egui::epaint::Shape::Text(t) => Some(format!("{} @ {:?}", t.galley.text(), t.pos)),
            _ => None,
        })
        .collect()
}
fn small() -> Fixture {
    Fixture::new(
        vec![
            Choice::new(10, "Noto Sans 日本語"),
            Choice::new(20, "DejaVu Serif"),
            Choice::new(30, "NOTO Serif"),
        ],
        20,
    )
}

#[test]
fn filters_without_committing_and_mouse_selection_closes_popup() {
    let mut f = small();
    let output = f.open();
    assert!(find(&output, "✓ DejaVu Serif").is_some());
    let output = f.query("nOtO");
    assert!(find(&output, "Noto Sans 日本語").is_some());
    assert!(find(&output, "NOTO Serif").is_some());
    assert!(find(&output, "✓ DejaVu Serif").is_none());
    assert_eq!(f.selected, 20);
    f.click(find(&output, "NOTO Serif").unwrap());
    assert_eq!(f.selected, 30);
    assert!(!f.state.is_open());
    let output = f.open();
    assert!(find(&output, "Noto Sans 日本語").is_some());
    assert!(find(&output, "✓ NOTO Serif").is_some());
    assert_eq!(f.selected, 30);
}

#[test]
fn empty_results_escape_and_outside_click_preserve_the_value() {
    let mut f = small();
    f.open();
    let output = f.query("missing");
    assert!(find(&output, "検索結果がありません").is_some());
    f.key(Key::Enter);
    assert!(f.state.is_open());
    assert_eq!(f.selected, 20);
    f.key(Key::Escape);
    assert!(!f.state.is_open());
    assert_eq!(f.selected, 20);
    f.open();
    f.query("日本");
    f.click(egui::pos2(550.0, 450.0));
    assert!(!f.state.is_open());
    assert_eq!(f.selected, 20);
}

#[test]
fn keyboard_navigation_scrolls_virtualized_rows_and_only_enter_commits() {
    let mut f = Fixture::new(
        (0..500)
            .map(|i| Choice::new(i, format!("Font {i:03}")))
            .collect(),
        400,
    );
    let output = f.open();
    assert!(
        find(&output, "✓ Font 400").is_some(),
        "{:?} {:?}",
        f.state,
        texts(&output)
    );
    assert!(
        output.shapes.len() < 100,
        "only visible rows should be painted"
    );
    for _ in 0..20 {
        f.key(Key::ArrowDown);
    }
    f.key(Key::ArrowUp);
    let output = f.frame(vec![]);
    let position = find(&output, "Font 419").expect("highlight must scroll into view");
    assert!(position.y < 350.0);
    assert_eq!(f.selected, 400);
    f.key(Key::Enter);
    assert_eq!(f.selected, 419);
    assert!(!f.state.is_open());
}

#[test]
fn mouse_wheel_scrolls_candidates_without_changing_selection() {
    let mut f = Fixture::new(
        (0..500)
            .map(|i| Choice::new(i, format!("Font {i:03}")))
            .collect(),
        0,
    );
    let output = f.open();
    let point = find(&output, "Font 004").unwrap();
    f.frame(vec![
        Event::PointerMoved(point),
        Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, -400.0),
            modifiers: Modifiers::NONE,
        },
    ]);
    for _ in 0..10 {
        f.frame(vec![]);
    }
    let output = f.frame(vec![]);
    assert!(find(&output, "✓ Font 000").is_none());
    assert_eq!(f.selected, 0);
    assert!(f.state.is_open());
    let (label, pos) = output
        .shapes
        .iter()
        .filter_map(|s| match &s.shape {
            egui::epaint::Shape::Text(t) if t.galley.text().starts_with("Font ") => {
                Some((t.galley.text(), t.pos + t.galley.size() / 2.0))
            }
            _ => None,
        })
        .find(|(label, pos)| !label.contains('▾') && pos.y > 80.0 && pos.y < 250.0)
        .expect("scrolled options must remain clickable");
    let value: usize = label.strip_prefix("Font ").unwrap().parse().unwrap();
    f.click(pos);
    assert_eq!(f.selected, value);
    assert!(!f.state.is_open());
}

#[test]
fn ime_conversion_enter_does_not_accept_an_option() {
    let mut f = small();
    f.open();
    f.frame(vec![
        Event::Ime(egui::ImeEvent::Enabled),
        Event::Ime(egui::ImeEvent::Preedit("にほん".into())),
        key_event(Key::Enter, true, Modifiers::NONE),
    ]);
    assert!(f.state.is_open());
    assert_eq!(f.selected, 20);
    f.frame(vec![key_event(Key::Enter, false, Modifiers::NONE)]);
    f.frame(vec![
        Event::Ime(egui::ImeEvent::Commit("日本".into())),
        key_event(Key::Enter, true, Modifiers::NONE),
    ]);
    assert!(f.state.is_open());
    assert_eq!(f.selected, 20);
    f.key(Key::Enter);
    assert_eq!(f.selected, 10);
    assert!(!f.state.is_open());
}

#[test]
fn unlisted_value_survives_open_close_and_empty_query_lists_every_option() {
    let mut f = small();
    f.selected = 999;
    f.open();
    assert_eq!(f.selected, 999);
    f.query("unknown");
    let output = f.query("");
    assert!(
        find(&output, "Noto Sans 日本語").is_some(),
        "{:?} {:?}",
        f.state,
        texts(&output)
    );
    assert!(find(&output, "DejaVu Serif").is_some());
    assert!(find(&output, "NOTO Serif").is_some());
    f.key(Key::Escape);
    assert_eq!(f.selected, 999);
}
