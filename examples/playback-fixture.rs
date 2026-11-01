//! Development-only synthetic native playback fixture. No external installation or ENGINE access.
use std::{fs, path::PathBuf};
use voxmoviemaker::{
    assets::Storage,
    media::ffprobe,
    project::{persistence, *},
    settings::{Settings, Theme},
};
fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let root = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .ok_or("provide a NEW absolute temporary directory")?,
    );
    if !root.is_absolute() {
        return Err("absolute directory required".into());
    }
    fs::create_dir(&root)?; // Never overwrite a pre-existing fixture or user files.
    let mut project = Project {
        width: 640,
        height: 360,
        ..Default::default()
    };
    for (name, color, track, start, duration) in [
        ("red", [255, 0, 0, 255], 0, 0, 90),
        ("blue", [0, 0, 255, 255], 1, 30, 60),
    ] {
        let path = root.join(format!("{name}.png"));
        let mut image = image::RgbaImage::from_pixel(640, 360, image::Rgba(color));
        if track == 1 {
            for y in 0..360 {
                for x in 0..320 {
                    image.put_pixel(x, y, image::Rgba([0, 0, 0, 0]));
                }
            }
        }
        image.save(&path)?;
        let asset = ffprobe::probe(std::path::Path::new("ffprobe"), &path)?;
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
    let path = root.join("test-tone.wav");
    let mut wav = hound::WavWriter::create(
        &path,
        hound::WavSpec {
            channels: 1,
            sample_rate: 48000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        },
    )?;
    for sample in 0..144_000 {
        wav.write_sample(
            ((sample as f64 * std::f64::consts::TAU * 440.0 / 48000.0).sin() * 2000.0) as i16,
        )?;
    }
    wav.finalize()?;
    let asset = ffprobe::probe(std::path::Path::new("ffprobe"), &path)?;
    project.tracks[2].clips.push(Clip {
        id: Id::new_v4(),
        source: ClipSource::Media { asset_id: asset.id },
        timeline_start: 0,
        duration: 90,
        source_offset: 0,
        volume: 1.0,
        muted: false,
        effects: Default::default(),
    });
    project.assets.push(asset);
    if std::env::args().any(|arg| arg == "--phase5") {
        project.tracks[1].clips[0].effects = ClipEffects {
            x: 60.0,
            scale: 0.8,
            opacity: 0.8,
            fade_in: 15,
            fade_out: 15,
            transition: TransitionKind::SlideLeft,
            transition_frames: 15,
            ..Default::default()
        };
        for (track, text, size, anchor) in [
            (3, "字幕・テキスト・フェード", 32.0, TextAnchor::Bottom),
            (4, "VoxMovieMaker\nPhase 5", 64.0, TextAnchor::Center),
        ] {
            project.tracks[track].clips.push(Clip {
                id: Id::new_v4(),
                source: ClipSource::Text {
                    settings: TextSettings {
                        text: text.into(),
                        size,
                        anchor,
                        subtitle: track == 3,
                        ..Default::default()
                    },
                },
                timeline_start: 0,
                duration: 90,
                source_offset: 0,
                volume: 1.0,
                muted: false,
                effects: ClipEffects {
                    fade_in: 15,
                    fade_out: 15,
                    ..Default::default()
                },
            });
        }
    }
    persistence::save(&root.join("playback.vmm.json"), &project)?;
    let english = std::env::args().any(|arg| arg == "--en");
    let chinese = std::env::args().any(|arg| arg == "--zh");
    Settings {
        preview_audio: false,
        language: if chinese {
            "zh-CN"
        } else if english {
            "en-US"
        } else {
            "ja-JP"
        }
        .into(),
        voice_height: 100.0,
        timeline_height: 330.0,
        theme: if english { Theme::Light } else { Theme::Dark },
        ..Default::default()
    }
    .save(&Storage {
        config: root.join("config/voxmoviemaker"),
        data: root.join("data/voxmoviemaker"),
        cache: root.join("cache/voxmoviemaker"),
    })?;
    println!("{}", root.display());
    Ok(())
}
