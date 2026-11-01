use eframe::egui;
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
};
use voxmoviemaker::{app::App, assets::Storage, settings::Settings};

fn load_fonts(ctx: &egui::Context) -> bool {
    let mut definitions = egui::FontDefinitions::empty();
    let system_font = Command::new("fc-match")
        .args(["-f", "%{file}", "sans:lang=ja"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| PathBuf::from(String::from_utf8_lossy(&o.stdout).trim()));
    let candidates = system_font.into_iter().chain([
        PathBuf::from("/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf"),
        PathBuf::from("/usr/share/fonts/TTF/DejaVuSans.ttf"),
        PathBuf::from("/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc"),
        PathBuf::from("/usr/share/fonts/truetype/droid/DroidSansFallbackFull.ttf"),
    ]);
    let mut loaded = HashSet::new();
    for (i, path) in candidates.enumerate() {
        if !loaded.insert(path.clone()) {
            continue;
        }
        if let Ok(bytes) = fs::read(path) {
            let name = format!("system-{i}");
            definitions
                .font_data
                .insert(name.clone(), Arc::new(egui::FontData::from_owned(bytes)));
            definitions
                .families
                .entry(egui::FontFamily::Proportional)
                .or_default()
                .push(name.clone());
            definitions
                .families
                .entry(egui::FontFamily::Monospace)
                .or_default()
                .push(name);
            if definitions.font_data.len() >= 2 {
                break;
            }
        }
    }
    if definitions.font_data.is_empty() {
        return false;
    }
    ctx.set_fonts(definitions);
    true
}
fn main() -> eframe::Result {
    // Read-only diagnostics also work without a display or external media tools.
    if std::env::args().any(|a| a == "--version") {
        println!("VoxMovieMaker {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    let storage = match Storage::discover() {
        Ok(storage) => storage,
        Err(error) => {
            eprintln!("VoxMovieMaker: {error}");
            return Ok(());
        }
    };
    let (settings, warning) = Settings::load(&storage);
    let mut storage = storage;
    if let Some(path) = settings.data_override.as_ref().filter(|p| p.is_absolute()) {
        storage.data = path.clone();
    }
    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Glow,
        viewport: egui::ViewportBuilder::default()
            .with_title("VoxMovieMaker")
            .with_inner_size(settings.window_size)
            .with_min_inner_size([800.0, 650.0]),
        ..Default::default()
    };
    // Publication owner/application ID intentionally remain undecided; no invented reverse DNS ID.
    eframe::run_native(
        "VoxMovieMaker",
        options,
        Box::new(move |cc| {
            if !load_fonts(&cc.egui_ctx) {
                return Err(std::io::Error::other("No system font found. Install a Japanese sans font manually (e.g. Noto Sans CJK).").into());
            }
            let mut app = App::new(cc, storage, settings, warning);
            let arguments: Vec<_> = std::env::args().collect();
            if let Some(pair) = arguments.windows(2).find(|a| a[0] == "--smoke-test") {
                app.smoke_screenshot = Some(pair[1].clone().into());
            }
            if let Some(pair) = arguments.windows(2).find(|a| a[0] == "--smoke-playback") {
                app.smoke_play_until = pair[1].parse().ok();
            }
            if let Some(path) = std::env::args().nth(1).filter(|p| !p.starts_with('-')) {
                match voxmoviemaker::project::persistence::load(Path::new(&path)) {
                    Ok(project) => {
                        app.project = project;
                        app.saved = app.project.clone();
                        app.project_path = Some(path.into());
                    }
                    Err(error) => app.status = app.local_error(&error),
                }
            }
            Ok(Box::new(app))
        }),
    )
}
