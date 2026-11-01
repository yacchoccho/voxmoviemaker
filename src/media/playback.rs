//! Bounded streaming playback of disposable, low-resolution timeline proxies.
use super::{audio::AudioDevice, ffmpeg, render_plan::RenderPlan};
use crate::{
    Result,
    assets::{Storage, private_directory, resolved_path},
    project::FrameRate,
};
use eframe::egui;
use std::{
    collections::hash_map::DefaultHasher,
    fs,
    hash::{Hash, Hasher},
    io::{BufRead, BufReader, Read},
    os::fd::AsRawFd,
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    process::Command,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
        mpsc::{self, Receiver, SyncSender, TrySendError},
    },
    thread,
    time::{Duration, Instant},
};

pub struct ProxyRequest {
    pub plan: RenderPlan,
    pub storage: Storage,
    pub ffmpeg: PathBuf,
    pub width: u32,
    pub limit_mb: u64,
    pub progress: Arc<AtomicU32>,
}
#[derive(Clone, Debug)]
pub struct Proxy {
    pub path: PathBuf,
    pub width: u32,
    pub height: u32,
    pub fps: FrameRate,
    pub duration: u64,
}

/// One full proxy at a time; limit includes the new file and all other owned proxies.
pub fn prepare(
    request: &ProxyRequest,
    cancelled: &dyn Fn() -> bool,
    repaint: impl Fn(),
) -> Result<Proxy> {
    if cancelled() {
        return Err("cancelled".into());
    }
    let directory = request.storage.cache.join("proxies");
    let data = resolved_path(&request.storage.data)?;
    let cache = resolved_path(&request.storage.cache)?;
    if cache.starts_with(&data) || data.starts_with(&cache) {
        return Err("data-cache-overlap".into());
    }
    if request.plan.duration == 0 {
        return Err("empty-project".into());
    }
    let (width, height) = request.plan.preview_size(request.width);
    let mut hash = DefaultHasher::new();
    "proxy-mpeg4-pcm-nut-v1".hash(&mut hash);
    serde_json::to_vec(&request.plan)
        .map_err(|e| e.to_string())?
        .hash(&mut hash);
    request.width.hash(&mut hash);
    request.ffmpeg.hash(&mut hash);
    for clip in &request.plan.clips {
        if clip.visual
            && let Some(text) = &clip.text
        {
            super::text::hash_font(text, &mut hash)?;
        }
    }
    for path in request
        .plan
        .clips
        .iter()
        .map(|clip| &clip.path)
        .chain(std::iter::once(&request.ffmpeg))
    {
        if let Ok(metadata) = fs::metadata(path) {
            metadata.len().hash(&mut hash);
            metadata.modified().ok().hash(&mut hash);
        }
    }
    private_directory(&directory)?;
    let path = directory.join(format!("vmm-proxy-{:016x}.nut", hash.finish()));
    let limit = request.limit_mb.clamp(1, 2048) * 1024 * 1024;
    super::cache::prune(&request.storage.cache.join("frames"), limit);
    let proxy = Proxy {
        path,
        width,
        height,
        fps: request.plan.fps,
        duration: request.plan.duration,
    };
    if proxy.path.is_file() && fs::metadata(&proxy.path).is_ok_and(|m| m.len() <= limit) {
        prune(&directory, limit, &proxy.path);
        request.progress.store(1000, Ordering::Relaxed);
        return Ok(proxy);
    }
    // Make room before writing. Only owned .nut files are disposable; WAV assets are elsewhere.
    let compiled = request.plan.compile_proxy(request.width)?;
    prune(&directory, 0, Path::new(""));
    let temporary = directory.join(format!("vmm-proxy-{}.part", uuid::Uuid::new_v4()));
    fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(&temporary)
        .map_err(|e| e.to_string())?;
    let duration_us = request.plan.fps.samples(request.plan.duration, 1_000_000);
    let progress = request.progress.clone();
    let result = ffmpeg::run_stream(
        Command::new(&request.ffmpeg)
            .args(compiled.args)
            .args([
                "-c:v",
                "mpeg4",
                "-q:v",
                "5",
                "-g",
                "12",
                "-bf",
                "0",
                "-c:a",
                "pcm_s16le",
                "-ar",
                "48000",
                "-ac",
                "2",
                "-r",
                &request.plan.fps.ffmpeg(),
                "-t",
                &request.plan.fps.timestamp(request.plan.duration),
                "-fs",
                &limit.to_string(),
                "-progress",
                "pipe:1",
                "-nostats",
                "-f",
                "nut",
                "-y",
            ])
            .arg(&temporary),
        Duration::from_secs(24 * 60 * 60),
        cancelled,
        move |stdout| {
            let mut last = 0u64;
            for line in BufReader::new(stdout)
                .lines()
                .map_while(std::result::Result::ok)
            {
                if let Some(value) = line
                    .strip_prefix("out_time_us=")
                    .and_then(|s| s.parse::<u64>().ok())
                {
                    last = last.max(value);
                    progress.store(
                        ((last as u128 * 1000 / duration_us.max(1) as u128).min(999)) as u32,
                        Ordering::Relaxed,
                    );
                }
            }
            last
        },
    );
    repaint();
    let result = result.and_then(|(last, _)| {
        if cancelled() {
            return Err("cancelled".into());
        }
        let size = fs::metadata(&temporary).map_err(|e| e.to_string())?.len();
        if size > limit || last.saturating_add(25) < duration_us {
            return Err("proxy-cache-full".into());
        }
        fs::rename(&temporary, &proxy.path).map_err(|e| e.to_string())?;
        request.progress.store(1000, Ordering::Relaxed);
        Ok(proxy)
    });
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}
pub(crate) fn prune(directory: &Path, limit: u64, keep: &Path) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    let mut files: Vec<_> = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_str()?;
            let metadata = entry.metadata().ok()?;
            (name.starts_with("vmm-proxy-") && name.ends_with(".nut") && metadata.is_file())
                .then(|| (metadata.modified().ok(), metadata.len(), path))
        })
        .collect();
    files.sort_by_key(|(modified, _, _)| *modified);
    let mut size: u64 = files.iter().map(|(_, len, _)| *len).sum();
    for (_, len, path) in files {
        if size <= limit {
            break;
        }
        if path != keep && fs::remove_file(path).is_ok() {
            size = size.saturating_sub(len);
        }
    }
}

pub fn frame_from_samples(fps: FrameRate, start: u64, samples: u64) -> u64 {
    start.saturating_add(
        (samples as u128 * fps.numerator as u128 / (48_000u128 * fps.denominator as u128)) as u64,
    )
}
pub struct VideoFrame {
    pub frame: u64,
    pub rgba: Vec<u8>,
}
struct Shared {
    audio: bool,
    cancel: AtomicBool,
    ready: AtomicBool,
    samples: AtomicU64,
    audio_ended: AtomicBool,
    video_ended: AtomicBool,
    silent_start: Mutex<Option<Instant>>,
    skipped: AtomicU64,
    audio_underruns: AtomicU64,
}
impl Shared {
    fn current_samples(&self) -> u64 {
        if self.audio {
            return self.samples.load(Ordering::Relaxed);
        }
        self.silent_start
            .lock()
            .ok()
            .and_then(|start| *start)
            .map_or(0, |start| {
                (start.elapsed().as_nanos() * 48_000 / 1_000_000_000).min(u64::MAX as u128) as u64
            })
    }
}
pub struct Session {
    pub proxy: Proxy,
    pub start: u64,
    pub frames: Receiver<VideoFrame>,
    pub errors: Receiver<String>,
    shared: Arc<Shared>,
    audio: bool,
}
impl Session {
    pub fn start(
        proxy: Proxy,
        binary: PathBuf,
        start: u64,
        device: Option<String>,
        ctx: egui::Context,
    ) -> Self {
        let start = start.min(proxy.duration.saturating_sub(1));
        let (sender, frames) = mpsc::sync_channel(2);
        let (error_sender, errors) = mpsc::sync_channel(2);
        let shared = Arc::new(Shared {
            audio: device.is_some(),
            cancel: AtomicBool::new(false),
            ready: AtomicBool::new(false),
            samples: AtomicU64::new(0),
            audio_ended: AtomicBool::new(false),
            video_ended: AtomicBool::new(false),
            silent_start: Mutex::new(None),
            skipped: AtomicU64::new(0),
            audio_underruns: AtomicU64::new(0),
        });
        let state = shared.clone();
        let source = proxy.clone();
        let executable = binary.clone();
        let errors_tx = error_sender.clone();
        let repaint = ctx.clone();
        thread::spawn(move || {
            let result = video(
                &source,
                &executable,
                start,
                sender,
                state.clone(),
                repaint.clone(),
            );
            match result {
                Err(error) if !state.cancel.load(Ordering::Relaxed) => {
                    let _ = errors_tx.try_send(error);
                    state.cancel.store(true, Ordering::Relaxed);
                }
                Ok(()) => state.video_ended.store(true, Ordering::Release),
                _ => {}
            }
            repaint.request_repaint();
        });
        let audio = device.is_some();
        if let Some(device) = device {
            let state = shared.clone();
            let source = proxy.clone();
            thread::spawn(move || {
                let result = audio_stream(&source, &binary, start, &device, state.clone());
                if let Err(error) = result
                    && !state.cancel.load(Ordering::Relaxed)
                {
                    let _ = error_sender.try_send(error);
                    state.cancel.store(true, Ordering::Relaxed);
                } else {
                    state.audio_ended.store(true, Ordering::Release);
                }
                ctx.request_repaint();
            });
        }
        Self {
            proxy,
            start,
            frames,
            errors,
            shared,
            audio,
        }
    }
    /// Start the clock only once the consumer has uploaded its first picture.
    /// Decoder startup, queued frames, and GUI preparation are not playback time.
    pub fn presented(&self) {
        if let Ok(mut start) = self.shared.silent_start.lock()
            && start.is_none()
        {
            *start = Some(Instant::now());
            self.shared.ready.store(true, Ordering::Release);
        }
    }
    pub fn frame(&self) -> u64 {
        let samples = self.shared.current_samples();
        frame_from_samples(self.proxy.fps, self.start, samples).min(self.proxy.duration)
    }
    pub fn played_samples(&self) -> u64 {
        self.shared.current_samples()
    }
    pub fn ended(&self) -> bool {
        self.shared.video_ended.load(Ordering::Acquire)
            && if self.audio {
                self.shared.audio_ended.load(Ordering::Acquire)
            } else {
                self.frame() >= self.proxy.duration
            }
    }
    pub fn skipped(&self) -> u64 {
        self.shared.skipped.load(Ordering::Relaxed)
    }
    pub fn audio_underruns(&self) -> u64 {
        self.shared.audio_underruns.load(Ordering::Relaxed)
    }
    pub fn cancel(&self) {
        self.shared.cancel.store(true, Ordering::Relaxed);
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        self.cancel();
    }
}

fn input(binary: &Path, proxy: &Proxy, start: u64) -> Command {
    let mut command = Command::new(binary);
    // Seek earlier, then trim on output. Demuxer seeking directly to the target can
    // skip an audio packet straddling that point (observed with NUT PCM).
    let preroll = proxy.fps.frames(2.0);
    let input_start = start.saturating_sub(preroll);
    command.args([
        "-hide_banner",
        "-nostdin",
        "-v",
        "error",
        "-threads",
        "1",
        "-protocol_whitelist",
        "file,pipe",
    ]);
    if input_start > 0 {
        command.args(["-ss", &proxy.fps.timestamp(input_start)]);
    }
    command
        .arg("-i")
        .arg(&proxy.path)
        .args(["-ss", &proxy.fps.timestamp(start - input_start)]);
    command
}
fn send_frame(sender: &SyncSender<VideoFrame>, mut frame: VideoFrame, state: &Shared) -> bool {
    loop {
        if state.cancel.load(Ordering::Relaxed) {
            return false;
        }
        match sender.try_send(frame) {
            Ok(()) => return true,
            Err(TrySendError::Disconnected(_)) => return false,
            Err(TrySendError::Full(back)) => frame = back,
        }
        thread::sleep(Duration::from_millis(2));
    }
}
fn video(
    proxy: &Proxy,
    binary: &Path,
    start: u64,
    sender: SyncSender<VideoFrame>,
    state: Arc<Shared>,
    ctx: egui::Context,
) -> Result<()> {
    let bytes = (proxy.width as usize)
        .checked_mul(proxy.height as usize)
        .and_then(|n| n.checked_mul(4))
        .filter(|n| *n >= 16 && *n <= 1280 * 720 * 4)
        .ok_or("preview-frame-invalid")?;
    let flag = state.clone();
    let fps = proxy.fps;
    let (result, _) = ffmpeg::run_stream(
        input(binary, proxy, start).args([
            "-map",
            "0:v:0",
            "-an",
            "-threads",
            "1",
            "-pix_fmt",
            "rgba",
            "-frames:v",
            &(proxy.duration - start).to_string(),
            "-f",
            "rawvideo",
            "pipe:1",
        ]),
        Duration::from_secs_f64(proxy.fps.seconds(proxy.duration - start) + 30.0),
        || flag.cancel.load(Ordering::Relaxed),
        move |mut stdout| -> Result<()> {
            let mut frame = start;
            loop {
                let mut rgba = vec![0; bytes];
                if stdout.read(&mut rgba[..1]).map_err(|e| e.to_string())? == 0 {
                    break;
                }
                stdout
                    .read_exact(&mut rgba[1..])
                    .map_err(|e| e.to_string())?;
                let due = frame_from_samples(fps, start, state.current_samples());
                if frame + 1 < due {
                    state.skipped.fetch_add(1, Ordering::Relaxed);
                } else {
                    if !send_frame(&sender, VideoFrame { frame, rgba }, &state) {
                        return Ok(());
                    }
                    ctx.request_repaint();
                }
                frame += 1;
            }
            if frame == start {
                return Err("preview-frame-invalid".into());
            }
            Ok(())
        },
    )?;
    result
}
fn audio_stream(
    proxy: &Proxy,
    binary: &Path,
    start: u64,
    device: &str,
    state: Arc<Shared>,
) -> Result<()> {
    static AUDIO_OWNER: Mutex<()> = Mutex::new(());
    let device = device.to_string();
    let flag = state.clone();
    let failure = Arc::new(Mutex::new(None));
    let failed = failure.clone();
    let result = ffmpeg::run_stream(
        input(binary, proxy, start).args([
            "-map",
            "0:a:0",
            "-vn",
            "-threads",
            "1",
            "-c:a",
            "pcm_s16le",
            "-ar",
            "48000",
            "-ac",
            "2",
            "-f",
            "s16le",
            "pipe:1",
        ]),
        Duration::from_secs_f64(proxy.fps.seconds(proxy.duration - start) + 30.0),
        || flag.cancel.load(Ordering::Relaxed),
        move |mut stdout| -> Result<()> {
            let result: Result<()> = (|| {
                // A seek can start the next decoder before the old audio thread
                // finishes. Wait off the UI thread before opening an exclusive PCM.
                let deadline = Instant::now() + Duration::from_secs(2);
                let _owner = loop {
                    if state.cancel.load(Ordering::Relaxed) {
                        return Ok(());
                    }
                    if let Ok(owner) = AUDIO_OWNER.try_lock() {
                        break owner;
                    }
                    if Instant::now() > deadline {
                        return Err("audio-device-unavailable".into());
                    }
                    thread::sleep(Duration::from_millis(2));
                };
                let mut output = AudioDevice::open(&device)?;
                let deadline = Instant::now() + Duration::from_secs(15);
                while !state.ready.load(Ordering::Acquire) {
                    if state.cancel.load(Ordering::Relaxed) {
                        return Ok(());
                    }
                    if Instant::now() > deadline {
                        return Err("process-timeout".into());
                    }
                    thread::sleep(Duration::from_millis(2));
                }
                // A pipe read must not prevent clock updates or cancellation
                // when decoding stalls. PCM memory stays independent of duration.
                let fd = stdout.as_raw_fd();
                // SAFETY: the owned pipe remains open throughout this callback.
                let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
                if flags < 0
                    || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0
                {
                    return Err(std::io::Error::last_os_error().to_string());
                }
                let mut buffer = [0; 7680]; // 40ms stereo S16_LE.
                loop {
                    if state.cancel.load(Ordering::Relaxed) {
                        return Ok(());
                    }
                    let mut filled = 0;
                    while filled < buffer.len() {
                        if state.cancel.load(Ordering::Relaxed) {
                            return Ok(());
                        }
                        let count = match stdout.read(&mut buffer[filled..]) {
                            Ok(count) => count,
                            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {
                                continue;
                            }
                            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                                state.samples.store(output.position(), Ordering::Relaxed);
                                thread::sleep(Duration::from_millis(2));
                                continue;
                            }
                            Err(error) => return Err(error.to_string()),
                        };
                        if count == 0 {
                            break;
                        }
                        filled += count;
                    }
                    if filled == 0 {
                        break;
                    }
                    if !filled.is_multiple_of(4) {
                        return Err("audio-frame-invalid".into());
                    }
                    let mut cursor = 0;
                    while cursor < filled {
                        if state.cancel.load(Ordering::Relaxed) {
                            return Ok(());
                        }
                        cursor += output.write(&buffer[cursor..filled])? * 4;
                        state.samples.store(output.position(), Ordering::Relaxed);
                        state
                            .audio_underruns
                            .store(output.underruns(), Ordering::Relaxed);
                        if cursor < filled {
                            thread::sleep(Duration::from_millis(2));
                        }
                    }
                }
                let end = output.submitted();
                output.start()?;
                let deadline = Instant::now() + Duration::from_secs(3);
                // Keep the output running with silence until its reported audible
                // position reaches the last MEDIA sample. An empty ALSA ring can
                // underrun while PipeWire still holds the tail; XRUN is not proof
                // that those samples were heard. snd_pcm_drain can block inside
                // plugins even in nonblocking mode, so use cancellable writes.
                let silence = [0; 7680];
                loop {
                    if state.cancel.load(Ordering::Relaxed) {
                        return Ok(());
                    }
                    if Instant::now() > deadline {
                        return Err("audio-playback-failed".into());
                    }
                    let position = output.position().min(end);
                    state.samples.store(position, Ordering::Relaxed);
                    if position == end {
                        break;
                    }
                    output.write(&silence)?;
                    state
                        .audio_underruns
                        .store(output.underruns(), Ordering::Relaxed);
                    thread::sleep(Duration::from_millis(2));
                }
                // Drop now discards only trailing silence. Cancellation above
                // still discards immediately and never waits for playback.
                state.samples.store(end, Ordering::Relaxed);
                Ok(())
            })();
            if let Err(error) = &result
                && let Ok(mut failure) = failed.lock()
            {
                *failure = Some(error.clone());
            }
            result
        },
    );
    if let Some(error) = failure.lock().ok().and_then(|mut error| error.take()) {
        return Err(error);
    }
    result.and_then(|(result, _)| result)
}
