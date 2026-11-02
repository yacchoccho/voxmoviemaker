use std::{fs, os::unix::ffi::OsStringExt, path::PathBuf, process::Command, time::Instant};
use voxmoviemaker::{
    assets::Storage,
    media::{ffprobe, render_plan::RenderPlan},
    project::{
        commands::{self, History},
        *,
    },
    settings::Settings,
};
struct Fixture {
    root: PathBuf,
    storage: Storage,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("vmm-review-{}", Id::new_v4()));
        fs::create_dir(&root).unwrap();
        Self {
            storage: Storage {
                data: root.join("data"),
                cache: root.join("cache"),
                config: root.join("config"),
            },
            root,
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
fn text_clip(start: u64) -> Clip {
    Clip {
        character: None,
        id: Id::new_v4(),
        source: ClipSource::Text {
            settings: TextSettings::default(),
        },
        timeline_start: start,
        duration: 1,
        source_offset: 0,
        volume: 1.0,
        muted: false,
        effects: Default::default(),
    }
}
fn image_project(f: &Fixture, count: u64) -> Project {
    let source = f.root.join("source.png");
    image::RgbaImage::from_pixel(16, 16, image::Rgba([255, 0, 0, 255]))
        .save(&source)
        .unwrap();
    let asset=ffprobe::parse(br#"{"streams":[{"index":0,"codec_type":"video","codec_name":"png","width":16,"height":16}],"format":{"format_name":"png_pipe"}}"#,&source).unwrap();
    let mut project = Project::default();
    for n in 0..count {
        let mut c = text_clip(n);
        c.source = ClipSource::Media { asset_id: asset.id };
        project.tracks[0].clips.push(c);
    }
    project.assets.push(asset);
    project
}
#[test]
fn large_filter_graph_launches_and_temporary_script_is_cleaned() {
    let f = Fixture::new();
    let project = image_project(&f, 1000);
    let plan = RenderPlan::new(&project, &f.storage).unwrap();
    for compiled in [
        plan.compile(None).unwrap(),
        plan.compile_proxy(160).unwrap(),
    ] {
        let script = compiled
            .args
            .windows(2)
            .find(|p| p[0] == "-filter_complex_script")
            .unwrap()[1]
            .clone();
        assert!(fs::read(&script).unwrap().len() > 128 * 1024);
        assert!(compiled.args.iter().all(|a| a.len() < 64 * 1024));
        assert!(
            Command::new("/bin/true")
                .args(&compiled.args)
                .status()
                .unwrap()
                .success()
        );
        assert!(PathBuf::from(&script).exists());
        drop(compiled);
        assert!(!PathBuf::from(script).exists());
    }
}
#[test]
#[ignore = "requires user-installed FFmpeg with libx264/aac"]
fn actual_ffmpeg_reads_large_filter_script() {
    use std::{
        sync::{Arc, atomic::AtomicBool},
        time::Duration,
    };
    use voxmoviemaker::{
        media::{export, ffmpeg},
        settings::ExportFormat,
    };
    let f = Fixture::new();
    let mut project = image_project(&f, 180);
    project.width = 16;
    project.height = 16;
    let plan = RenderPlan::new(&project, &f.storage).unwrap();
    let settings = Settings::default();
    let caps = ffmpeg::detect(&settings).unwrap();
    export::export(
        &plan,
        &settings.ffmpeg,
        &caps,
        ExportFormat::Mp4,
        &f.root.join("output.mp4"),
        Arc::new(AtomicBool::new(false)),
        |_| {},
    )
    .unwrap();
    let out = ffmpeg::run(
        Command::new(&settings.ffprobe)
            .args([
                "-v",
                "error",
                "-show_entries",
                "format=duration",
                "-of",
                "csv=p=0",
            ])
            .arg(f.root.join("output.mp4")),
        Duration::from_secs(10),
        || false,
    )
    .unwrap();
    let duration = String::from_utf8(out.stdout)
        .unwrap()
        .trim()
        .parse::<f64>()
        .unwrap();
    assert!((duration - 6.0).abs() < 0.1);
    assert_eq!(fs::read_dir(&plan.scratch).unwrap().count(), 0);
}
#[test]
fn non_utf8_media_is_rejected_before_external_process_or_project_mutation() {
    let f = Fixture::new();
    let path = f
        .root
        .join(std::ffi::OsString::from_vec(b"image-\xff.png".to_vec()));
    fs::write(&path, b"source").unwrap();
    assert_eq!(
        ffprobe::probe(std::path::Path::new("/does-not-exist"), &path).unwrap_err(),
        "non-utf8-path"
    );
    let mut project = image_project(&f, 1);
    project.assets[0].location = AssetLocation::External(path);
    assert_eq!(project.validate().unwrap_err(), "non-utf8-path");
    assert!(persistence::save(&f.root.join("invalid.vmm.json"), &project).is_err());
    assert!(!f.root.join("invalid.vmm.json").exists());
}
#[test]
fn rejected_or_legacy_invalid_data_override_does_not_survive_load() {
    let f = Fixture::new();
    let settings = Settings {
        data_override: Some(f.storage.cache.clone()),
        ..Default::default()
    };
    settings.save(&f.storage).unwrap();
    let (loaded, warning) = Settings::load(&f.storage);
    assert!(loaded.data_override.is_none());
    assert!(warning.is_some());
    assert!(f.storage.validate_data_root(&f.storage.data).is_ok());
}
#[test]
fn validation_handles_unsorted_boundaries_and_large_tracks() {
    let mut project = Project::default();
    project.tracks[0].clips = (0..10_000).rev().map(text_clip).collect();
    let before = project.clone();
    let start = Instant::now();
    project.validate().unwrap();
    eprintln!("10,000 clips validated in {:?}", start.elapsed());
    assert_eq!(project, before, "validation must not reorder stored clips");
    project.tracks[0].clips[0].timeline_start = 1;
    assert!(project.validate().is_err());
    project.tracks[0].clips = vec![text_clip(MAX_FRAME - 1)];
    project.validate().unwrap();
    project.tracks[0].clips[0].duration = 2;
    assert!(project.validate().is_err());
    project.tracks[0].clips[0].timeline_start = u64::MAX;
    assert!(project.validate().is_err());
}
#[test]
fn grouped_edits_keep_one_delta_and_restore_redo() {
    let mut project = Project::default();
    project.tracks[0].clips = (0..1000).map(text_clip).collect();
    let initial = project.clone();
    let mut history = History::default();
    history
        .edit(&mut project, |p| {
            p.tracks[1].name = "earlier edit".into();
            Ok(())
        })
        .unwrap();
    let before = project.clone();
    history.begin_group(&project);
    for n in 1..=100 {
        history
            .edit(&mut project, |p| {
                p.tracks[0].clips[500].effects.x = n as f64;
                Ok(())
            })
            .unwrap();
    }
    history.finish_group(&project).unwrap();
    assert_eq!(history.undo_len(), 2);
    assert!(
        history.retained_bytes() < 16 * 1024,
        "unchanged 1,000 clips must not be retained per edit"
    );
    let after = project.clone();
    history.undo(&mut project);
    assert_eq!(project, before);
    history.undo(&mut project);
    assert_eq!(project, initial);
    history.redo(&mut project);
    history.redo(&mut project);
    assert_eq!(project, after);
}
#[test]
fn delta_history_roundtrips_tracks_assets_moves_and_budget_eviction() {
    let f = Fixture::new();
    let mut project = image_project(&f, 3);
    let original = project.clone();
    let mut history = History::default();
    let first = project.tracks[0].clips[0].id;
    history
        .edit(&mut project, |p| commands::move_clip(p, first, 1, 10))
        .unwrap();
    let moved = project.clone();
    history
        .edit(&mut project, |p| {
            p.tracks.swap(0, 1);
            Ok(())
        })
        .unwrap();
    history
        .edit(&mut project, |p| {
            p.tracks.remove(4);
            Ok(())
        })
        .unwrap();
    let after = project.clone();
    for _ in 0..3 {
        history.undo(&mut project);
    }
    assert_eq!(project, original);
    history.redo(&mut project);
    assert_eq!(project, moved);
    history.redo(&mut project);
    history.redo(&mut project);
    assert_eq!(project, after);
    history.undo(&mut project);
    history
        .edit(&mut project, |p| {
            p.tracks.push(Track::new(6));
            Ok(())
        })
        .unwrap();
    let branched = project.clone();
    history.redo(&mut project);
    assert_eq!(project, branched);
    let mut limited = History::with_limits(50, 4096);
    for n in 0..100 {
        limited
            .edit(&mut project, |p| {
                p.tracks[0].name = format!("name {n}");
                Ok(())
            })
            .unwrap();
        assert!(limited.retained_bytes() <= 4096);
    }
    assert!(limited.undo_len() < 50);
    let mut tiny = History::with_limits(50, 1);
    tiny.edit(&mut project, |p| {
        p.width = 1920;
        Ok(())
    })
    .unwrap();
    let after = project.clone();
    tiny.undo(&mut project);
    assert_eq!(project, after);
}
