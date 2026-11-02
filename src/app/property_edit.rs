use super::App;
use crate::{
    Result,
    project::{Id, Project},
};
use eframe::egui;

pub struct PropertyEdit {
    target: Id,
    focus: Option<egui::Id>,
}
impl App {
    pub fn finish_property_edit(&mut self) {
        if self.property_edit.take().is_some() {
            let result = self.history.finish_group(&self.project);
            self.report(result);
        }
    }
    pub fn finish_idle_property_edit(&mut self, ctx: &egui::Context) {
        if let Some(edit) = &self.property_edit {
            let active = if let Some(focus) = edit.focus {
                ctx.memory(|m| m.has_focus(focus))
            } else {
                ctx.input(|i| i.pointer.primary_down())
            };
            if !active {
                self.finish_property_edit();
            }
        }
    }
    pub fn edit_property(
        &mut self,
        ctx: &egui::Context,
        target: Id,
        edit: impl FnOnce(&mut Project) -> Result<()>,
    ) {
        if self.property_edit.as_ref().is_some_and(|p| {
            p.target != target || p.focus.is_some_and(|id| !ctx.memory(|m| m.has_focus(id)))
        }) {
            self.finish_property_edit();
        }
        if self.property_edit.is_none() {
            self.finish_preview_transform();
            let focus = if ctx.input(|i| i.pointer.primary_down()) {
                None
            } else {
                ctx.memory(|m| m.focused())
            };
            self.property_edit = Some(PropertyEdit { target, focus });
            self.history.begin_group(&self.project);
        }
        let result = self.history.edit(&mut self.project, edit);
        if self.report(result).is_some() {
            self.changed();
        }
    }
}
