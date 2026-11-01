use super::{App, ExportJob};
use crate::project::portability;
use eframe::egui;
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};
impl App {
    pub fn start_package(&mut self, target: PathBuf, ctx: &egui::Context) {
        self.finish_preview_transform();
        if self.package.is_some() {
            return;
        }
        self.pause_timeline();
        let project = self.project.clone();
        let storage = self.storage.clone();
        let ctx = ctx.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let (sender, receiver) = mpsc::channel();
        let (updates, progress) = mpsc::sync_channel(8);
        thread::spawn(move || {
            let result = portability::export(
                &project,
                &storage,
                &target,
                || flag.load(Ordering::Relaxed),
                |value| {
                    let _ = updates.try_send(value);
                    ctx.request_repaint();
                },
            );
            let _ = sender.send(result);
            ctx.request_repaint();
        });
        self.package = Some(ExportJob {
            cancel,
            receiver,
            progress,
            value: 0.0,
        });
    }
    pub fn poll_package(&mut self, ctx: &egui::Context) {
        let finished = self.package.as_mut().and_then(|job| {
            for value in job.progress.try_iter() {
                job.value = value;
            }
            job.receiver.try_recv().ok()
        });
        if let Some(result) = finished {
            self.package = None;
            if self.report(result).is_some() {
                self.status = self.tr("package-complete");
            }
        }
        if self.package.is_some() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }
}
