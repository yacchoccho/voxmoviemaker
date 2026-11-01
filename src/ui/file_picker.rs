//! Optional OS chooser, with a Rust folder browser when Zenity is unavailable.
//! Neither Zenity nor its libraries are linked, installed, or distributed by this app.
use crate::{
    Result, app::PathMode, i18n::I18n, media::ffmpeg::protect_child, project::Id,
    tasks::LatestWorker,
};
use eframe::egui;
use std::{
    fs,
    io::Read,
    os::unix::ffi::OsStringExt,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::mpsc::{self, Receiver},
    thread,
    time::Duration,
};

pub struct FilePicker {
    pub mode: PathMode,
    pub project: Id,
    pub track: Id,
    pub start: u64,
    native: Option<NativePicker>,
    browser: Browser,
}
struct NativePicker {
    child: Child,
    output: Receiver<Vec<u8>>,
}
impl Drop for NativePicker {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
struct Entry {
    path: PathBuf,
    directory: bool,
}
type Listing = Result<Vec<Entry>>;
struct Browser {
    directory: PathBuf,
    entries: Listing,
    worker: Option<LatestWorker<(PathBuf, bool), Vec<Entry>>>,
    ticket: u64,
    loading: bool,
    selected: Option<PathBuf>,
    hidden: bool,
    loaded: bool,
    file_name: String,
}
impl Browser {
    fn new() -> Self {
        let directory = std::env::var_os("HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute() && p.is_dir())
            .unwrap_or_else(|| PathBuf::from("/"));
        Self {
            directory,
            entries: Ok(Vec::new()),
            worker: None,
            ticket: 0,
            loading: false,
            selected: None,
            hidden: false,
            loaded: false,
            file_name: String::new(),
        }
    }
    fn navigate(&mut self, path: PathBuf, ctx: &egui::Context) {
        self.loaded = true;
        self.directory = path.clone();
        self.selected = None;
        self.entries = Ok(Vec::new());
        if self.worker.is_none() {
            self.worker = Some(LatestWorker::with_processor(
                ctx.clone(),
                |(path, hidden): (PathBuf, bool), cancelled| {
                    let mut entries = Vec::new();
                    for entry in fs::read_dir(path).map_err(|e| e.to_string())? {
                        if cancelled() {
                            return Err("cancelled".into());
                        }
                        let entry = entry.map_err(|e| e.to_string())?;
                        if !hidden && entry.file_name().as_encoded_bytes().starts_with(b".") {
                            continue;
                        }
                        if entries.len() >= 10_000 {
                            return Err("directory-too-large".into());
                        }
                        let path = entry.path();
                        entries.push(Entry {
                            directory: path.is_dir(),
                            path,
                        });
                    }
                    entries.sort_by(|a, b| b.directory.cmp(&a.directory).then(a.path.cmp(&b.path)));
                    Ok(entries)
                },
            ));
        }
        self.ticket = self
            .worker
            .as_ref()
            .expect("initialized directory worker")
            .request((path, self.hidden));
        self.loading = true;
    }
}
impl FilePicker {
    pub fn new(mode: PathMode, project: Id, track: Id, start: u64, title: &str) -> Self {
        let native = native_picker(title, None);
        Self {
            mode,
            project,
            track,
            start,
            native,
            browser: Browser::new(),
        }
    }
    pub fn built_in(mode: PathMode, project: Id, track: Id, start: u64) -> Self {
        Self {
            mode,
            project,
            track,
            start,
            native: None,
            browser: Browser::new(),
        }
    }
    pub fn save(mode: PathMode, project: Id, title: &str, initial: PathBuf) -> Self {
        let mut picker = Self::built_in_save(mode, project, initial);
        let filename = picker.browser.directory.join(&picker.browser.file_name);
        picker.native = native_picker(title, Some(&filename));
        picker
    }
    pub fn built_in_save(mode: PathMode, project: Id, initial: PathBuf) -> Self {
        debug_assert!(matches!(mode, PathMode::Save | PathMode::Export));
        let mut picker = Self::built_in(mode, project, Id::nil(), 0);
        if let Some(parent) = initial.parent().filter(|p| p.is_absolute() && p.is_dir()) {
            picker.browser.directory = parent.to_path_buf();
        }
        picker.browser.file_name = initial
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        picker
    }
    /// Also used if the system chooser fails or the user requests the built-in browser.
    pub fn browse(&mut self, directory: PathBuf, ctx: &egui::Context) {
        self.native = None;
        self.browser.navigate(directory, ctx);
    }
    pub fn show(&mut self, ctx: &egui::Context, i18n: &I18n) -> Option<Result<Option<PathBuf>>> {
        let saving = matches!(self.mode, PathMode::Save | PathMode::Export);
        let title = match self.mode {
            PathMode::Save => "choose-project-path",
            PathMode::Export => "choose-export-path",
            _ => "choose-media",
        };
        if let Some(native) = &mut self.native {
            match native.child.try_wait() {
                Ok(Some(status)) if status.success() => match native.output.try_recv() {
                    Ok(bytes) => return Some(parse_selection(bytes, saving).map(Some)),
                    Err(mpsc::TryRecvError::Disconnected) => {
                        return Some(Err("invalid-path".into()));
                    }
                    Err(mpsc::TryRecvError::Empty) => {}
                },
                Ok(Some(status)) if status.code() == Some(1) => return Some(Ok(None)),
                Ok(Some(_)) | Err(_) => {
                    self.native = None;
                    self.browser.navigate(self.browser.directory.clone(), ctx);
                }
                Ok(None) => {}
            }
        }
        if self.native.is_some() {
            ctx.request_repaint_after(Duration::from_millis(100));
            let mut cancel = false;
            let mut fallback = false;
            egui::Window::new(i18n.text(title))
                .id(egui::Id::new("native-media-picker"))
                .order(egui::Order::Foreground)
                .collapsible(false)
                .show(ctx, |ui| {
                    ui.label(i18n.text("system-picker-open"));
                    fallback = ui.button(i18n.text("built-in-picker")).clicked();
                    cancel = ui.button(i18n.text("cancel")).clicked();
                });
            if cancel {
                return Some(Ok(None));
            }
            if fallback {
                self.browse(self.browser.directory.clone(), ctx);
            }
            return None;
        }
        if !self.browser.loaded {
            self.browser.navigate(self.browser.directory.clone(), ctx);
        }
        if let Some(worker) = &self.browser.worker {
            while let Ok((ticket, entries)) = worker.receiver.try_recv() {
                if ticket == self.browser.ticket {
                    self.browser.entries = entries;
                    self.browser.loading = false;
                }
            }
        }
        let mut open = true;
        let mut next = None;
        let mut result = None;
        egui::Window::new(i18n.text(title))
            .id(egui::Id::new("built-in-media-picker"))
            .order(egui::Order::Foreground)
            .open(&mut open)
            .default_size([650.0, 450.0])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if ui.button(i18n.text("home-folder")).clicked() {
                        next = Some(Browser::new().directory);
                    }
                    if ui
                        .add_enabled(
                            self.browser.directory.parent().is_some(),
                            egui::Button::new(i18n.text("parent-folder")),
                        )
                        .clicked()
                    {
                        next = self.browser.directory.parent().map(PathBuf::from);
                    }
                    if ui.button(i18n.text("refresh")).clicked() {
                        next = Some(self.browser.directory.clone());
                    }
                    if ui
                        .checkbox(&mut self.browser.hidden, i18n.text("show-hidden"))
                        .changed()
                    {
                        next = Some(self.browser.directory.clone());
                    }
                });
                ui.label(self.browser.directory.to_string_lossy());
                if self.browser.loading {
                    ui.spinner();
                }
                match &self.browser.entries {
                    Ok(entries) => {
                        egui::ScrollArea::vertical().max_height(300.0).show_rows(
                            ui,
                            24.0,
                            entries.len(),
                            |ui, rows| {
                                for row in rows {
                                    let entry = &entries[row];
                                    let name = entry
                                        .path
                                        .file_name()
                                        .unwrap_or_default()
                                        .to_string_lossy();
                                    let label = if entry.directory {
                                        format!("{name}/")
                                    } else {
                                        name.into_owned()
                                    };
                                    let hit = ui.selectable_label(
                                        self.browser.selected.as_ref() == Some(&entry.path),
                                        label,
                                    );
                                    if hit.clicked() && !entry.directory {
                                        self.browser.selected = Some(entry.path.clone());
                                        if saving {
                                            self.browser.file_name = entry
                                                .path
                                                .file_name()
                                                .unwrap_or_default()
                                                .to_string_lossy()
                                                .into_owned();
                                        }
                                    }
                                    if hit.double_clicked() {
                                        if entry.directory {
                                            next = Some(entry.path.clone());
                                        } else if !saving {
                                            result = Some(Ok(Some(entry.path.clone())));
                                        }
                                    }
                                }
                            },
                        );
                    }
                    Err(error) => {
                        ui.colored_label(
                            ui.visuals().error_fg_color,
                            format!("{}: {}", i18n.text("folder-read-error"), i18n.text(error)),
                        );
                    }
                }
                if saving {
                    ui.label(i18n.text("export-file-name"));
                    ui.add(
                        egui::TextEdit::singleline(&mut self.browser.file_name)
                            .desired_width(f32::INFINITY),
                    );
                    ui.small(i18n.text(if self.mode == PathMode::Save {
                        "project-extension"
                    } else {
                        "export-no-overwrite"
                    }));
                } else {
                    ui.small(i18n.text("picker-hint"));
                }
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(
                            if saving {
                                valid_file_name(&self.browser.file_name)
                            } else {
                                self.browser.selected.is_some()
                            },
                            egui::Button::new(i18n.text(if saving {
                                "select-export-path"
                            } else {
                                "select-file"
                            })),
                        )
                        .clicked()
                    {
                        result = Some(Ok(if saving {
                            Some(self.browser.directory.join(&self.browser.file_name))
                        } else {
                            self.browser.selected.clone()
                        }));
                    }
                    if ui.button(i18n.text("cancel")).clicked() {
                        result = Some(Ok(None));
                    }
                });
            });
        if !open {
            return Some(Ok(None));
        }
        if let Some(path) = next {
            self.browser.navigate(path, ctx);
        }
        result
    }
}
fn native_picker(title: &str, filename: Option<&Path>) -> Option<NativePicker> {
    let mut command = Command::new("zenity");
    command
        .args(["--file-selection", "--title", title])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if let Some(filename) = filename {
        command.args(["--save", "--filename"]).arg(filename);
    }
    protect_child(&mut command);
    command.spawn().ok().and_then(|mut child| {
        let Some(stdout) = child.stdout.take() else {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        };
        let (sender, output) = mpsc::channel();
        thread::spawn(move || {
            let mut bytes = Vec::new();
            let _ = stdout.take(65_537).read_to_end(&mut bytes);
            let _ = sender.send(bytes);
        });
        Some(NativePicker { child, output })
    })
}
fn valid_file_name(name: &str) -> bool {
    !name.trim().is_empty() && !name.contains(['/', '\0']) && name != "." && name != ".."
}
pub fn project_filename(mut path: PathBuf) -> PathBuf {
    if !path.as_os_str().as_encoded_bytes().ends_with(b".vmm.json") {
        let mut filename = path.file_name().unwrap_or_default().to_os_string();
        filename.push(".vmm.json");
        path.set_file_name(filename);
    }
    path
}
pub fn export_filename(mut path: PathBuf, format: crate::settings::ExportFormat) -> PathBuf {
    let extension = match format {
        crate::settings::ExportFormat::Mp4 => "mp4",
        crate::settings::ExportFormat::WebM => "webm",
    };
    if path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case(extension))
    {
        return path;
    }
    if path.extension().is_none()
        || path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("mp4") || e.eq_ignore_ascii_case("webm"))
    {
        path.set_extension(extension);
    } else {
        let mut filename = path.file_name().unwrap_or_default().to_os_string();
        filename.push(format!(".{extension}"));
        path.set_file_name(filename);
    }
    path
}
fn parse_selection(mut bytes: Vec<u8>, saving: bool) -> Result<PathBuf> {
    // Zenity prints one terminating newline. Do not trim spaces/newlines belonging to a name.
    if bytes.last() == Some(&b'\n') {
        bytes.pop();
    }
    if bytes.is_empty() || bytes.len() > 65_536 || bytes.contains(&0) {
        return Err("invalid-path".into());
    }
    let path = PathBuf::from(std::ffi::OsString::from_vec(bytes));
    if !path.is_absolute()
        || if saving {
            path.file_name().is_none()
                || path.is_dir()
                || !path.parent().is_some_and(|p| p.is_dir())
        } else {
            !path.is_file()
        }
    {
        return Err("invalid-path".into());
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::ffi::OsStrExt;

    #[test]
    fn native_selection_preserves_japanese_spaces_newlines_and_non_utf8_names() {
        let root = std::env::temp_dir().join(format!("vmm-picker-{}", Id::new_v4()));
        fs::create_dir(&root).unwrap();
        for name in ["日本語 の画像\n.png".as_bytes(), b"non-utf8-\xff.wav"] {
            let path = root.join(std::ffi::OsString::from_vec(name.to_vec()));
            fs::write(&path, b"fixture").unwrap();
            let mut output = path.as_os_str().as_bytes().to_vec();
            output.push(b'\n');
            assert_eq!(parse_selection(output, false).unwrap(), path);
        }
        assert!(parse_selection(Vec::new(), false).is_err());
        assert!(parse_selection(b"relative-file.png\n".to_vec(), false).is_err());
        assert!(parse_selection(root.as_os_str().as_bytes().to_vec(), false).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn save_selection_accepts_new_files_and_keeps_paths_and_extensions() {
        use crate::settings::ExportFormat;
        let root = std::env::temp_dir().join(format!("vmm-save-picker-{}", Id::new_v4()));
        fs::create_dir(&root).unwrap();
        for name in ["日本語 の動画\n.mp4".as_bytes(), b"non-utf8-\xff.mp4"] {
            let path = root.join(std::ffi::OsString::from_vec(name.to_vec()));
            let mut bytes = path.as_os_str().as_bytes().to_vec();
            bytes.push(b'\n');
            assert_eq!(parse_selection(bytes.clone(), true).unwrap(), path);
            assert!(parse_selection(bytes, false).is_err());
            assert!(!path.exists(), "choosing a destination must not create it");
        }
        assert!(parse_selection(root.as_os_str().as_bytes().to_vec(), true).is_err());
        assert!(parse_selection(b"relative.mp4\n".to_vec(), true).is_err());
        assert_eq!(
            export_filename(root.join("movie"), ExportFormat::Mp4),
            root.join("movie.mp4")
        );
        assert_eq!(
            export_filename(root.join("movie.mp4"), ExportFormat::WebM),
            root.join("movie.webm")
        );
        assert_eq!(
            export_filename(root.join("movie.MP4"), ExportFormat::Mp4),
            root.join("movie.MP4")
        );
        assert_eq!(
            export_filename(root.join("movie.v1"), ExportFormat::Mp4),
            root.join("movie.v1.mp4")
        );
        assert_eq!(
            project_filename(root.join("編集した 動画")),
            root.join("編集した 動画.vmm.json")
        );
        assert_eq!(
            project_filename(root.join("project.vmm.json")),
            root.join("project.vmm.json")
        );
        let non_utf8 = root.join(std::ffi::OsString::from_vec(b"project-\xff".to_vec()));
        let expected = root.join(std::ffi::OsString::from_vec(
            b"project-\xff.vmm.json".to_vec(),
        ));
        assert_eq!(project_filename(non_utf8), expected);
        for name in ["", " ", ".", "..", "../bad", "/bad", "bad\0name"] {
            assert!(!valid_file_name(name));
        }
        fs::remove_dir_all(root).unwrap();
    }
}
