use crate::{
    Result,
    assets::Storage,
    media::{
        playback::{Proxy, ProxyRequest},
        render_plan::RenderPlan,
    },
};
use eframe::egui;
use std::{
    path::PathBuf,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    thread,
};

type Job<T> = Box<dyn FnOnce() -> T + Send>;
pub struct Worker<T> {
    sender: SyncSender<Job<T>>,
    pub receiver: Receiver<T>,
}
impl<T: Send + 'static> Worker<T> {
    pub fn new(ctx: egui::Context) -> Self {
        let (sender, jobs) = mpsc::sync_channel::<Job<T>>(8);
        let (results, receiver) = mpsc::sync_channel(8);
        thread::spawn(move || {
            while let Ok(job) = jobs.recv() {
                if results.send(job()).is_err() {
                    break;
                }
                ctx.request_repaint();
            }
        });
        Self { sender, receiver }
    }
    pub fn submit(&self, job: impl FnOnce() -> T + Send + 'static) -> Result<()> {
        self.sender
            .try_send(Box::new(job))
            .map_err(|_| "worker-busy".into())
    }
}
pub struct PreviewRequest {
    pub project: crate::project::Id,
    pub selected: Option<crate::project::Id>,
    pub plan: RenderPlan,
    pub storage: Storage,
    pub ffmpeg: PathBuf,
    pub frame: u64,
    pub width: u32,
    pub limit_mb: u64,
}
type Pending<R> = Option<(u64, R)>;
pub struct LatestWorker<R, T> {
    pending: Arc<(Mutex<Pending<R>>, Condvar)>,
    generation: Arc<AtomicU64>,
    shutdown: Arc<AtomicBool>,
    pub receiver: Receiver<(u64, Result<T>)>,
}
pub type PreviewWorker = LatestWorker<PreviewRequest, crate::media::preview::PreviewFrame>;
pub type ProxyWorker = LatestWorker<ProxyRequest, Proxy>;
impl PreviewWorker {
    pub fn new(ctx: egui::Context) -> Self {
        Self::with_processor(ctx, crate::media::preview::render)
    }
}
impl ProxyWorker {
    pub fn new(ctx: egui::Context) -> Self {
        let repaint = ctx.clone();
        Self::with_processor(ctx, move |request, cancelled| {
            super::media::playback::prepare(&request, cancelled, || repaint.request_repaint())
        })
    }
}
impl<R: Send + 'static, T: Send + 'static> LatestWorker<R, T> {
    pub(crate) fn with_processor(
        ctx: egui::Context,
        process: impl Fn(R, &dyn Fn() -> bool) -> Result<T> + Send + 'static,
    ) -> Self {
        let pending: Arc<(Mutex<Pending<R>>, Condvar)> =
            Arc::new((Mutex::new(None), Condvar::new()));
        let generation = Arc::new(AtomicU64::new(0));
        let shutdown = Arc::new(AtomicBool::new(false));
        let (sender, receiver) = mpsc::sync_channel(2);
        let slot = pending.clone();
        let ticket = generation.clone();
        let stopping = shutdown.clone();
        thread::spawn(move || {
            loop {
                let (id, request) = {
                    let Ok(mut guard) = slot.0.lock() else {
                        break;
                    };
                    while guard.is_none() && !stopping.load(Ordering::Relaxed) {
                        let Ok(next) = slot.1.wait(guard) else {
                            return;
                        };
                        guard = next;
                    }
                    if stopping.load(Ordering::Relaxed) {
                        return;
                    }
                    let Some(request) = guard.take() else {
                        continue;
                    };
                    request
                };
                let cancelled =
                    || ticket.load(Ordering::Relaxed) != id || stopping.load(Ordering::Relaxed);
                let mut value = (id, process(request, &cancelled));
                while !cancelled() {
                    ctx.request_repaint();
                    match sender.try_send(value) {
                        Ok(()) | Err(mpsc::TrySendError::Disconnected(_)) => break,
                        Err(mpsc::TrySendError::Full(back)) => value = back,
                    }
                    thread::sleep(std::time::Duration::from_millis(5));
                }
            }
        });
        Self {
            pending,
            generation,
            shutdown,
            receiver,
        }
    }
    pub fn request(&self, request: R) -> u64 {
        while self.receiver.try_recv().is_ok() {}
        let id = self.generation.fetch_add(1, Ordering::Relaxed) + 1;
        if let Ok(mut slot) = self.pending.0.lock() {
            *slot = Some((id, request));
            self.pending.1.notify_one();
        }
        id
    }
    pub fn cancel(&self) {
        self.generation.fetch_add(1, Ordering::Relaxed);
    }
}
impl<R, T> Drop for LatestWorker<R, T> {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        self.generation.fetch_add(1, Ordering::Relaxed);
        // Hold the mutex while notifying to avoid a missed wakeup at shutdown.
        if let Ok(mut slot) = self.pending.0.lock() {
            *slot = None;
            self.pending.1.notify_one();
        }
    }
}
