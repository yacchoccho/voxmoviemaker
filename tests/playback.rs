use eframe::egui;
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
use voxmoviemaker::{
    assets::Storage,
    media::{
        ffmpeg, ffprobe,
        playback::{self, Proxy, ProxyRequest, Session},
        render_plan::RenderPlan,
    },
    project::*,
    settings::Settings,
};

struct Fixture {
    root: PathBuf,
    storage: Storage,
    settings: Settings,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("vmm-playback-{}", Id::new_v4()));
        fs::create_dir(&root).unwrap();
        Self {
            storage: Storage {
                data: root.join("data"),
                cache: root.join("cache"),
                config: root.join("config"),
            },
            root,
            settings: Settings {
                ffmpeg: std::env::var_os("VMM_TEST_FFMPEG")
                    .unwrap_or_else(|| "ffmpeg".into())
                    .into(),
                ffprobe: std::env::var_os("VMM_TEST_FFPROBE")
                    .unwrap_or_else(|| "ffprobe".into())
                    .into(),
                ..Default::default()
            },
        }
    }
    fn project(&self) -> Project {
        let mut project = Project {
            width: 160,
            height: 160,
            ..Default::default()
        };
        for (name, color, track, start, duration) in [
            ("red", [255, 0, 0, 255], 0, 0, 60),
            ("blue", [0, 0, 255, 255], 1, 30, 30),
        ] {
            let path = self.root.join(format!("{name}.png"));
            let mut image = image::RgbaImage::from_pixel(32, 32, image::Rgba(color));
            if track == 1 {
                for x in 0..16 {
                    for y in 0..32 {
                        image.put_pixel(x, y, image::Rgba([0, 0, 0, 0]));
                    }
                }
            }
            image.save(&path).unwrap();
            let asset = ffprobe::probe(&self.settings.ffprobe, &path).unwrap();
            project.tracks[track].clips.push(Clip {
                id: Id::new_v4(),
                source: ClipSource::Media { asset_id: asset.id },
                timeline_start: start,
                duration,
                source_offset: 0,
                volume: 1.0,
                muted: false,
                effects: Default::default(),
            });
            project.assets.push(asset);
        }
        let path = self.root.join("tone.wav");
        let mut wav = hound::WavWriter::create(
            &path,
            hound::WavSpec {
                channels: 1,
                sample_rate: 48000,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            },
        )
        .unwrap();
        for sample in 0..192_000 {
            wav.write_sample(
                ((sample as f64 * std::f64::consts::TAU * 440.0 / 48_000.0).sin() * 2000.0) as i16,
            )
            .unwrap();
        }
        wav.finalize().unwrap();
        let asset = ffprobe::probe(&self.settings.ffprobe, &path).unwrap();
        project.tracks[2].clips.push(Clip {
            id: Id::new_v4(),
            source: ClipSource::Voicevox {
                asset_id: asset.id,
                text: "synthetic test tone".into(),
                settings: VoicevoxSettings::default(),
                subtitle: None,
            },
            timeline_start: 0,
            duration: 60,
            source_offset: 0,
            volume: 1.0,
            muted: false,
            effects: Default::default(),
        });
        project.assets.push(asset);
        project
    }
    fn request(&self, project: &Project) -> ProxyRequest {
        ProxyRequest {
            plan: RenderPlan::new(project, &self.storage).unwrap(),
            storage: self.storage.clone(),
            ffmpeg: self.settings.ffmpeg.clone(),
            width: 160,
            limit_mb: 8,
            progress: Arc::new(AtomicU32::new(0)),
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn audio_sample_clock_keeps_ntsc_boundaries_without_accumulating_error() {
    for fps in [
        FrameRate::default(),
        FrameRate {
            numerator: 24000,
            denominator: 1001,
        },
        FrameRate {
            numerator: 30000,
            denominator: 1001,
        },
    ] {
        for frame in [1u64, 30, 10_000, 1_000_000] {
            let boundary = (frame as u128 * 48000 * fps.denominator as u128)
                .div_ceil(fps.numerator as u128) as u64;
            assert_eq!(playback::frame_from_samples(fps, 17, boundary), frame + 17);
            assert_eq!(
                playback::frame_from_samples(fps, 17, boundary - 1),
                frame + 16
            );
        }
    }
}

#[test]
fn old_settings_keep_audio_defaults() {
    let settings: Settings =
        serde_json::from_str(r#"{"language":"en-US","preview_width":320}"#).unwrap();
    assert!(settings.preview_audio);
    assert_eq!(settings.audio_device, "default");
}

#[test]
fn streaming_decoder_backpressure_cancel_reaps_child_without_draining_frames() {
    let f = Fixture::new();
    let binary = f.root.join("decoder");
    fs::write(
        &binary,
        "#!/bin/sh\nprintf '%s' \"$$\" > \"${0}.pid\"\nexec head -c 100000000 /dev/zero\n",
    )
    .unwrap();
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
    let proxy = Proxy {
        path: f.root.join("unused.nut"),
        width: 2,
        height: 2,
        fps: FrameRate::default(),
        duration: 9000,
    };
    let session = Session::start(proxy, binary.clone(), 60, None, egui::Context::default());
    let first = session.frames.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_eq!((first.frame, first.rgba.len()), (60, 16));
    thread::sleep(Duration::from_millis(100)); // bounded channel is now full
    let pid = fs::read_to_string(binary.with_extension("pid")).unwrap();
    let process = PathBuf::from(format!("/proc/{pid}"));
    assert!(process.exists());
    session.cancel();
    let deadline = Instant::now() + Duration::from_secs(2);
    while process.exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(5));
    }
    assert!(
        !process.exists(),
        "cancel must kill and wait even with a full frame channel"
    );
}

#[test]
fn proxy_rejects_persistent_data_cache_overlap_before_spawning() {
    let f = Fixture::new();
    let mut request = f.request(&Project::default());
    request.storage.data = request.storage.cache.clone();
    assert_eq!(
        playback::prepare(&request, &|| false, || {}).unwrap_err(),
        "data-cache-overlap"
    );
    assert!(!request.storage.cache.exists());
}

#[test]
#[ignore = "requires user-installed FFmpeg/ffprobe with mpeg4, pcm_s16le and NUT"]
fn real_proxy_streams_layers_seeks_reuses_cache_and_skips_late_frames() {
    let f = Fixture::new();
    let project = f.project();
    let request = f.request(&project);
    let proxy = playback::prepare(&request, &|| false, || {}).unwrap();
    assert_eq!(request.progress.load(Ordering::Relaxed), 1000);
    assert_eq!((proxy.width, proxy.height, proxy.duration), (160, 160, 60));
    let modified = fs::metadata(&proxy.path).unwrap().modified().unwrap();
    assert_eq!(
        playback::prepare(&request, &|| false, || {}).unwrap().path,
        proxy.path
    );
    assert_eq!(
        fs::metadata(&proxy.path).unwrap().modified().unwrap(),
        modified
    );
    let session = Session::start(
        proxy.clone(),
        f.settings.ffmpeg.clone(),
        0,
        None,
        egui::Context::default(),
    );
    let first = session.frames.recv_timeout(Duration::from_secs(3)).unwrap();
    assert_eq!(first.frame, 0);
    session.presented();
    assert!(first.rgba[0] > 200 && first.rgba[2] < 40);
    thread::sleep(Duration::from_millis(500));
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut reached = false;
    while Instant::now() < deadline {
        if let Ok(error) = session.errors.try_recv() {
            panic!("{error}");
        }
        if let Ok(frame) = session.frames.recv_timeout(Duration::from_millis(50))
            && frame.frame >= 30
        {
            reached = true;
            break;
        }
    }
    assert!(reached);
    assert!(session.skipped() > 0);
    assert!(session.frame() >= 15);
    session.cancel();
    let seek = Session::start(
        proxy,
        f.settings.ffmpeg.clone(),
        30,
        None,
        egui::Context::default(),
    );
    let first = seek.frames.recv_timeout(Duration::from_secs(3)).unwrap();
    assert_eq!(first.frame, 30);
    seek.presented();
    let pixel = |x: usize| &first.rgba[(80 * 160 + x) * 4..][..4];
    assert!(pixel(40)[0] > 200 && pixel(40)[2] < 40);
    assert!(pixel(120)[2] > 200 && pixel(120)[0] < 40);
    seek.cancel();
}

#[test]
#[ignore = "requires user-installed FFmpeg/ffprobe with mpeg4, pcm_s16le and NUT"]
fn real_proxy_cancellation_and_capacity_leave_no_partial_cache() {
    let f = Fixture::new();
    let mut project = f.project();
    let mut request = f.request(&project);
    assert_eq!(
        playback::prepare(&request, &|| true, || {}).unwrap_err(),
        "cancelled"
    );
    assert!(!f.storage.cache.exists());
    // PCM alone exceeds 1MiB, even though its source tone is only four seconds long.
    project.tracks[0].clips[0].duration = 600;
    request = f.request(&project);
    request.limit_mb = 1;
    assert_eq!(
        playback::prepare(&request, &|| false, || {}).unwrap_err(),
        "proxy-cache-full"
    );
    assert!(
        fs::read_dir(f.storage.cache.join("proxies"))
            .unwrap()
            .next()
            .is_none()
    );
}

#[test]
#[ignore = "requires system ALSA null PCM and user-installed FFmpeg/ffprobe"]
fn real_proxy_audio_master_reaches_end_through_alsa_null_sink() {
    for fps in [
        FrameRate::default(),
        FrameRate {
            numerator: 24000,
            denominator: 1001,
        },
        FrameRate {
            numerator: 30000,
            denominator: 1001,
        },
    ] {
        let f = Fixture::new();
        let mut project = f.project();
        project.frame_rate = fps;
        project.tracks[0].clips[0].duration = 90;
        project.tracks[1].clips[0].duration = 60;
        project.tracks[2].clips[0].duration = 90;
        let proxy = playback::prepare(&f.request(&project), &|| false, || {}).unwrap();
        for start in [30, 75] {
            let session = Session::start(
                proxy.clone(),
                f.settings.ffmpeg.clone(),
                start,
                Some("null".into()),
                egui::Context::default(),
            );
            let _first = session.frames.recv_timeout(Duration::from_secs(3)).unwrap();
            session.presented();
            let deadline = Instant::now() + Duration::from_secs(4);
            while !session.ended() && Instant::now() < deadline {
                if let Ok(error) = session.errors.try_recv() {
                    panic!("{error}");
                }
                while session.frames.try_recv().is_ok() {}
                thread::sleep(Duration::from_millis(2));
            }
            if let Ok(error) = session.errors.try_recv() {
                panic!("{error}");
            }
            assert!(session.ended());
            let expected = fps.samples(90 - start, 48000);
            assert!(
                session.played_samples().abs_diff(expected) <= 1,
                "fps={fps:?}, samples={} expected={expected}",
                session.played_samples()
            );
            session.cancel();
        }
    }
}

#[test]
#[ignore = "plays a quiet test tone through the user's configured ALSA default device"]
fn real_system_audio_clock_paces_video_after_seek() {
    let f = Fixture::new();
    let proxy = playback::prepare(&f.request(&f.project()), &|| false, || {}).unwrap();
    let session = Session::start(
        proxy,
        f.settings.ffmpeg.clone(),
        15,
        Some("default".into()),
        egui::Context::default(),
    );
    let _first = session.frames.recv_timeout(Duration::from_secs(3)).unwrap();
    session.presented();
    let deadline = Instant::now() + Duration::from_secs(4);
    while session.played_samples() == 0 && Instant::now() < deadline {
        if let Ok(error) = session.errors.try_recv() {
            panic!("{error}");
        }
        while session.frames.try_recv().is_ok() {}
        thread::sleep(Duration::from_millis(2));
    }
    assert!(session.played_samples() > 0);
    let start = Instant::now();
    let first_sample = session.played_samples();
    let mut previous = first_sample;
    while start.elapsed() < Duration::from_millis(500) {
        if let Ok(error) = session.errors.try_recv() {
            panic!("{error}");
        }
        while session.frames.try_recv().is_ok() {}
        let current = session.played_samples();
        assert!(current >= previous);
        previous = current;
        thread::sleep(Duration::from_millis(5));
    }
    let played = (session.played_samples() - first_sample) as f64 / 48000.0;
    let elapsed = start.elapsed().as_secs_f64();
    assert!(
        (played - elapsed).abs() < 0.15,
        "audio clock advanced {played}s in {elapsed}s"
    );
    assert!(session.frame() >= 28);
    session.cancel();
    let resumed = Session::start(
        session.proxy.clone(),
        f.settings.ffmpeg.clone(),
        30,
        Some("default".into()),
        egui::Context::default(),
    );
    let _first = resumed.frames.recv_timeout(Duration::from_secs(3)).unwrap();
    resumed.presented();
    let deadline = Instant::now() + Duration::from_secs(3);
    while resumed.played_samples() == 0 && Instant::now() < deadline {
        if let Ok(error) = resumed.errors.try_recv() {
            panic!("{error}");
        }
        while resumed.frames.try_recv().is_ok() {}
        thread::sleep(Duration::from_millis(2));
    }
    assert!(resumed.played_samples() > 0);
    resumed.cancel();
    thread::sleep(Duration::from_millis(100));
    let paused = session.played_samples();
    thread::sleep(Duration::from_millis(50));
    assert_eq!(session.played_samples(), paused);
}

#[test]
#[ignore = "plays a quiet test tone through the user's configured ALSA default device"]
fn real_system_audio_plays_the_full_tail_before_natural_completion() {
    let f = Fixture::new();
    let proxy = playback::prepare(&f.request(&f.project()), &|| false, || {}).unwrap();
    let decoded = decode_proxy_audio(&f, &proxy);
    for start in [0, 45, 59] {
        // ALSA tee records only this application's submitted PCM, not the
        // microphone or desktop mix. Keep the real default device as its slave.
        let capture = f.root.join(format!("submitted-{start}.pcm"));
        let session = Session::start(
            proxy.clone(),
            f.settings.ffmpeg.clone(),
            start,
            Some(format!("tee:default,{}", capture.display())),
            egui::Context::default(),
        );
        let _first = session.frames.recv_timeout(Duration::from_secs(3)).unwrap();
        session.presented();
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut reference = None;
        while !session.ended() && Instant::now() < deadline {
            if let Ok(error) = session.errors.try_recv() {
                panic!("{error}");
            }
            while session.frames.try_recv().is_ok() {}
            let samples = session.played_samples();
            if samples > 0 && reference.is_none() {
                reference = Some((Instant::now(), samples));
            }
            thread::sleep(Duration::from_millis(2));
        }
        assert!(session.ended(), "start={start}");
        assert_eq!(session.audio_underruns(), 0, "start={start}: audio starved");
        let expected = proxy.fps.samples(proxy.duration - start, 48_000);
        assert!(session.played_samples().abs_diff(expected) <= 1);
        let written = fs::read(&capture).unwrap();
        let media = &decoded[proxy.fps.samples(start, 48_000) as usize * 4..];
        assert!(written.len() >= media.len());
        assert_eq!(
            &written[..media.len()],
            media,
            "PCM samples were lost, duplicated or changed"
        );
        assert!(
            written[media.len()..].iter().all(|byte| *byte == 0),
            "only silence may follow the media"
        );
        let (began, first_sample) = reference.expect("audible clock must advance");
        let remaining = (expected - first_sample) as f64 / 48_000.0;
        assert!(
            began.elapsed().as_secs_f64() + 0.01 >= remaining,
            "start={start}: ended in {:?}, but {remaining}s of audio remained",
            began.elapsed()
        );
    }
}

fn decode_proxy_audio(f: &Fixture, proxy: &Proxy) -> Vec<u8> {
    ffmpeg::run(
        Command::new(&f.settings.ffmpeg)
            .args(["-hide_banner", "-nostdin", "-v", "error", "-i"])
            .arg(&proxy.path)
            .args([
                "-map", "0:a:0", "-vn", "-ar", "48000", "-ac", "2", "-f", "s16le", "pipe:1",
            ]),
        Duration::from_secs(5),
        || false,
    )
    .unwrap()
    .stdout
}

#[test]
#[ignore = "plays a quiet test tone through the user's configured ALSA default device"]
fn real_audio_buffer_survives_a_short_writer_scheduling_stall() {
    let f = Fixture::new();
    let proxy = playback::prepare(&f.request(&f.project()), &|| false, || {}).unwrap();
    let decoded = decode_proxy_audio(&f, &proxy);
    let capture = f.root.join("scheduled-output.pcm");
    let mut output = voxmoviemaker::media::audio::AudioDevice::open(&format!(
        "tee:default,{}",
        capture.display()
    ))
    .unwrap();
    let mut cursor = 0;
    let mut stalled = false;
    let deadline = Instant::now() + Duration::from_secs(6);
    while cursor < decoded.len() && Instant::now() < deadline {
        let end = (cursor + 7680).min(decoded.len());
        let count = output.write(&decoded[cursor..end]).unwrap();
        cursor += count * 4;
        let played = output.position();
        if count == 0 {
            if played > 0 && !stalled {
                // Pause the WRITER itself after filling the active PCM ring.
                // A pipe full of decoded audio cannot hide this scheduling gap.
                stalled = true;
                thread::sleep(Duration::from_millis(140));
            } else {
                thread::sleep(Duration::from_millis(2));
            }
        }
    }
    assert!(stalled);
    assert_eq!(cursor, decoded.len());
    assert_eq!(output.underruns(), 0);
    // The separate Session end test checks audible completion. This test stops
    // explicitly after submitting the waveform, then checks the submitted bytes.
    drop(output);
    assert_eq!(fs::read(capture).unwrap(), decoded);
}

#[test]
fn preview_clock_does_not_spend_playback_time_before_the_first_frame_is_presented() {
    let f = Fixture::new();
    let binary = f.root.join("decoder");
    fs::write(&binary, "#!/bin/sh\nexec head -c 480 /dev/zero\n").unwrap();
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
    let session = Session::start(
        Proxy {
            path: f.root.join("unused.nut"),
            width: 2,
            height: 2,
            fps: FrameRate::default(),
            duration: 30,
        },
        binary,
        0,
        None,
        egui::Context::default(),
    );
    let first = session.frames.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_eq!(first.frame, 0);
    // Simulate the GUI needing 0.2s to prepare/upload its first frame.
    thread::sleep(Duration::from_millis(200));
    assert_eq!(
        session.frame(),
        0,
        "Decoding ahead must not start the visible playback clock"
    );
    assert!(!session.ended());
    let began = Instant::now();
    session.presented();
    let deadline = began + Duration::from_secs(3);
    let mut last = first.frame;
    while !session.ended() && Instant::now() < deadline {
        if let Ok(error) = session.errors.try_recv() {
            panic!("{error}");
        }
        while let Ok(frame) = session.frames.try_recv() {
            last = frame.frame;
        }
        thread::sleep(Duration::from_millis(2));
    }
    while let Ok(frame) = session.frames.try_recv() {
        last = frame.frame;
    }
    assert!(session.ended());
    assert!(
        began.elapsed() >= Duration::from_millis(990),
        "Preparation must not shorten the 1s clip"
    );
    assert_eq!(last, 29);
    assert_eq!(session.frame(), 30);
}

#[test]
#[ignore = "requires user-installed FFmpeg/ffprobe with mpeg4, PCM and NUT"]
fn real_silent_proxy_keeps_full_duration_and_last_frame_after_presentation_and_seek() {
    for fps in [
        FrameRate::default(),
        FrameRate {
            numerator: 24000,
            denominator: 1001,
        },
        FrameRate {
            numerator: 30000,
            denominator: 1001,
        },
    ] {
        let f = Fixture::new();
        let mut project = f.project();
        project.frame_rate = fps;
        project.tracks[1].clips[0].timeline_start = 54;
        project.tracks[1].clips[0].duration = 6;
        let proxy = playback::prepare(&f.request(&project), &|| false, || {}).unwrap();
        for start in [0, 45] {
            let session = Session::start(
                proxy.clone(),
                f.settings.ffmpeg.clone(),
                start,
                None,
                egui::Context::default(),
            );
            let first = session.frames.recv_timeout(Duration::from_secs(3)).unwrap();
            assert_eq!(first.frame, start);
            thread::sleep(Duration::from_millis(200));
            assert_eq!(session.frame(), start);
            let began = Instant::now();
            session.presented();
            let mut last = first;
            let deadline = began + Duration::from_secs(5);
            while !session.ended() && Instant::now() < deadline {
                if let Ok(error) = session.errors.try_recv() {
                    panic!("{error}");
                }
                while let Ok(frame) = session.frames.try_recv() {
                    last = frame;
                }
                thread::sleep(Duration::from_millis(2));
            }
            while let Ok(frame) = session.frames.try_recv() {
                last = frame;
            }
            assert!(session.ended(), "fps={fps:?}, start={start}");
            assert_eq!(session.frame(), 60);
            assert_eq!(last.frame, 59, "fps={fps:?}, start={start}");
            let pixel = &last.rgba[(80 * 160 + 120) * 4..][..4];
            assert!(
                pixel[2] > 200 && pixel[0] < 40,
                "The final blue segment must be decoded: {pixel:?}"
            );
            assert!(
                began.elapsed().as_secs_f64() + 0.01 >= fps.seconds(60 - start),
                "fps={fps:?}, start={start}, elapsed={:?}",
                began.elapsed()
            );
        }
    }
}
