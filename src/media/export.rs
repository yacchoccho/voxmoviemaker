use super::{
    ffmpeg::{Capabilities, OwnedProcess, ProcessReader, drain, is_cancelled},
    render_plan::RenderPlan,
};
use crate::{Result, settings::ExportFormat};
use std::{
    fs,
    io::{BufRead, BufReader},
    path::Path,
    process::{Command, Stdio},
    sync::{Arc, atomic::AtomicBool, mpsc},
    thread,
    time::{Duration, Instant},
};
use uuid::Uuid;

pub fn export(
    plan: &RenderPlan,
    binary: &Path,
    capabilities: &Capabilities,
    format: ExportFormat,
    target: &Path,
    cancel: Arc<AtomicBool>,
    progress: impl Fn(f32),
) -> Result<()> {
    if target.exists() {
        return Err("output-exists".into());
    }
    if plan.clips.is_empty() {
        return Err("empty-project".into());
    }
    let (video, audio, muxer) = match format {
        ExportFormat::Mp4 => ("libx264", "aac", "mp4"),
        ExportFormat::WebM => ("libvpx-vp9", "libopus", "webm"),
    };
    if !capabilities.supports_encoder(video)
        || !capabilities.supports_encoder(audio)
        || !capabilities.supports_muxer(muxer)
    {
        return Err("encoder-unavailable".into());
    }
    let parent = target
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let temporary = parent.join(format!(".voxmoviemaker-export-{}.part", Uuid::new_v4()));
    // Reserve only our temporary name. The destination is published without overwriting.
    fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)
        .map_err(|e| e.to_string())?;
    let result = (|| {
        let compiled = plan.compile(None)?;
        let mut command = Command::new(binary);
        command.args(compiled.args).args([
            "-c:v",
            video,
            "-c:a",
            audio,
            "-r",
            &plan.fps.ffmpeg(),
            "-t",
            &plan.fps.timestamp(plan.duration),
            "-progress",
            "pipe:1",
            "-nostats",
        ]);
        match format {
            ExportFormat::Mp4 => {
                command.args([
                    "-preset",
                    "veryfast",
                    "-crf",
                    "23",
                    "-movflags",
                    "+faststart",
                ]);
            }
            ExportFormat::WebM => {
                command.args([
                    "-deadline",
                    "good",
                    "-cpu-used",
                    "4",
                    "-crf",
                    "32",
                    "-b:v",
                    "0",
                ]);
            }
        }
        command.args(["-f", muxer, "-y"]).arg(&temporary);
        super::ffmpeg::protect_child(&mut command);
        let mut child = OwnedProcess::new(
            command
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .map_err(|e| e.to_string())?,
        );
        let stdout = child.stdout.take().ok_or("process-pipe")?;
        let stderr = child.stderr.take().ok_or("process-pipe")?;
        let stdout = ProcessReader::new(stdout, cancel.clone()).map_err(|e| e.to_string())?;
        let stderr = ProcessReader::new(stderr, cancel.clone()).map_err(|e| e.to_string())?;
        let (tx, rx) = mpsc::sync_channel(16);
        let reader = thread::spawn(move || {
            for line in BufReader::new(stdout)
                .lines()
                .map_while(std::result::Result::ok)
            {
                if let Some(value) = line
                    .strip_prefix("out_time_us=")
                    .and_then(|s| s.parse::<u64>().ok())
                {
                    let _ = tx.try_send(value);
                }
            }
        });
        let errors = thread::spawn(move || drain(stderr, 256 * 1024));
        let started = Instant::now();
        let mut aborted = false;
        let status = loop {
            for value in rx.try_iter() {
                progress(
                    (value as f64 / 1_000_000.0 / plan.fps.seconds(plan.duration)).clamp(0.0, 0.999)
                        as f32,
                );
            }
            if is_cancelled(&cancel) || started.elapsed() > Duration::from_secs(24 * 3600) {
                aborted = true;
                cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                child.terminate();
                break child.wait();
            }
            match child.try_wait() {
                Ok(Some(status)) => break Ok(status),
                Ok(None) => thread::sleep(Duration::from_millis(50)),
                Err(e) => {
                    cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                    child.terminate();
                    break Err(e);
                }
            }
        };
        child.terminate();
        let _ = reader.join();
        let stderr = errors.join().map_err(|_| "process-reader")?;
        if aborted {
            return Err("cancelled".into());
        }
        if !status.map_err(|e| e.to_string())?.success() {
            return Err(format!(
                "export-failed: {}",
                String::from_utf8_lossy(&stderr)
            ));
        }
        let file = fs::File::open(&temporary).map_err(|e| e.to_string())?;
        if file.metadata().map_err(|e| e.to_string())?.len() == 0 {
            return Err("empty-output".into());
        }
        file.sync_all().map_err(|e| e.to_string())?;
        // hard_link fails if the final name already exists, including a racing writer.
        fs::hard_link(&temporary, target).map_err(|e| e.to_string())?;
        if let Ok(directory) = fs::File::open(parent) {
            let _ = directory.sync_all();
        }
        progress(1.0);
        Ok(())
    })();
    let _ = fs::remove_file(&temporary);
    result
}
