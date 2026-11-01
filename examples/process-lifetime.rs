//! Development-only probe: the spawned sleep must be killed when this parent exits.
//! This example does not run or control VOICEVOX.
use std::process::{Command, Stdio};
// Intentionally leave the child unwaited to exercise parent-death cleanup.
// Production process runners always wait/reap normally.
#[allow(clippy::zombie_processes)]
fn main() {
    let mut command = Command::new("sleep");
    command
        .arg("60")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    voxmoviemaker::media::ffmpeg::protect_child(&mut command);
    let child = command.spawn().expect("spawn lifetime probe");
    println!("{}", child.id());
}
