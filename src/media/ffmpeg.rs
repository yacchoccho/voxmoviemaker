use crate::{Result, settings::Settings};
use std::{
    io::Read,
    process::{ChildStdout, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

pub struct Output {
    pub stdout: Vec<u8>,
    pub stderr: String,
}
/// Only for processes spawned by this app, never a user-started ENGINE.
pub fn protect_child(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    let parent = std::process::id() as libc::pid_t;
    // SAFETY: the post-fork closure only makes Linux system calls and constructs
    // errno errors. It allocates nothing and takes no locks before exec.
    unsafe {
        command.pre_exec(move || {
            if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL, 0, 0, 0) == -1 {
                return Err(std::io::Error::last_os_error());
            }
            if libc::getppid() != parent {
                return Err(std::io::Error::from_raw_os_error(libc::ESRCH));
            }
            Ok(())
        });
    }
}
pub(crate) fn drain(mut input: impl Read, limit: usize) -> Vec<u8> {
    let mut kept = Vec::new();
    let mut buffer = [0; 8192];
    while let Ok(n) = input.read(&mut buffer) {
        if n == 0 {
            break;
        }
        kept.extend_from_slice(&buffer[..n.min(limit.saturating_sub(kept.len()))]);
    }
    kept
}
/// Drain both pipes concurrently, cap retained output, reap on cancellation/timeout.
pub fn run(
    command: &mut Command,
    timeout: Duration,
    cancelled: impl Fn() -> bool,
) -> Result<Output> {
    let (stdout, stderr) = run_stream(command, timeout, cancelled, |stdout| {
        drain(stdout, 16 * 1024 * 1024)
    })?;
    Ok(Output { stdout, stderr })
}
pub fn run_stream<T: Send + 'static>(
    command: &mut Command,
    timeout: Duration,
    cancelled: impl Fn() -> bool,
    read_stdout: impl FnOnce(ChildStdout) -> T + Send + 'static,
) -> Result<(T, String)> {
    protect_child(command);
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    let stdout = child.stdout.take().ok_or("process-pipe")?;
    let stderr = child.stderr.take().ok_or("process-pipe")?;
    let out = thread::spawn(move || read_stdout(stdout));
    let err = thread::spawn(move || drain(stderr, 256 * 1024));
    let start = Instant::now();
    let mut failure = None;
    let status = loop {
        if cancelled() || start.elapsed() > timeout {
            failure = Some(
                if cancelled() {
                    "cancelled"
                } else {
                    "process-timeout"
                }
                .to_string(),
            );
            let _ = child.kill();
            break child.wait();
        }
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) => thread::sleep(Duration::from_millis(15)),
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                break Err(e);
            }
        }
    };
    let stdout = out.join().map_err(|_| "process-reader")?;
    let stderr = String::from_utf8_lossy(&err.join().map_err(|_| "process-reader")?).to_string();
    if let Some(failure) = failure {
        return Err(failure);
    }
    if !status.map_err(|e| e.to_string())?.success() {
        return Err(format!("process-failed: {stderr}"));
    }
    Ok((stdout, stderr))
}
#[derive(Clone, Debug, Default)]
pub struct Capabilities {
    pub ffmpeg: String,
    pub ffprobe: String,
    pub encoders: String,
    pub formats: String,
}
impl Capabilities {
    pub fn supports_encoder(&self, encoder: &str) -> bool {
        self.encoders
            .lines()
            .any(|l| l.split_whitespace().nth(1) == Some(encoder))
    }
    pub fn supports_muxer(&self, muxer: &str) -> bool {
        self.formats.lines().any(|l| {
            let parts: Vec<_> = l.split_whitespace().collect();
            parts.len() > 1 && parts[0].contains('E') && parts[1].split(',').any(|m| m == muxer)
        })
    }
}
pub fn detect(settings: &Settings) -> Result<Capabilities> {
    let get = |binary: &std::path::Path, flag: &str| -> Result<String> {
        let result = run(
            Command::new(binary).args(["-hide_banner", flag]),
            Duration::from_secs(10),
            || false,
        )?;
        Ok(format!(
            "{}{}",
            String::from_utf8_lossy(&result.stdout),
            result.stderr
        ))
    };
    Ok(Capabilities {
        ffmpeg: get(&settings.ffmpeg, "-version")?
            .lines()
            .next()
            .unwrap_or_default()
            .into(),
        ffprobe: get(&settings.ffprobe, "-version")?
            .lines()
            .next()
            .unwrap_or_default()
            .into(),
        encoders: get(&settings.ffmpeg, "-encoders")?,
        formats: get(&settings.ffmpeg, "-formats")?,
    })
}
pub fn cancellation() -> Arc<AtomicBool> {
    Arc::new(AtomicBool::new(false))
}
pub fn is_cancelled(flag: &AtomicBool) -> bool {
    flag.load(Ordering::Relaxed)
}
