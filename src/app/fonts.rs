use super::{App, Event};
use crate::{
    media::fonts::FontId,
    ui::searchable_combo_box::{Choice, SearchState},
};

#[derive(Default)]
pub struct FontCatalog {
    pub choices: Vec<Choice<FontId>>,
    pub search: SearchState,
    pub loading: bool,
    pub error: Option<String>,
    pub loaded: bool,
}
impl App {
    pub fn request_fonts(&mut self) {
        if self.fonts.loading {
            return;
        }
        match self
            .worker
            .submit(|| Event::Fonts(crate::media::fonts::list()))
        {
            Ok(()) => {
                self.fonts.loading = true;
                self.fonts.error = None;
            }
            Err(error) => self.fonts.error = Some(error),
        }
    }
}
