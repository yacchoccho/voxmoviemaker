use crate::{
    Result,
    assets::{Storage, private_directory},
    project::persistence::{atomic_write, read_limited},
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum Theme {
    Dark,
    Light,
    #[default]
    System,
}
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum ExportFormat {
    #[default]
    Mp4,
    WebM,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub language: String,
    pub theme: Theme,
    pub ffmpeg: PathBuf,
    pub ffprobe: PathBuf,
    pub voicevox_url: String,
    pub allow_remote_voicevox: bool,
    pub preview_width: u32,
    pub preview_audio: bool,
    pub audio_device: String,
    pub cache_limit_mb: u64,
    pub export_format: ExportFormat,
    pub window_size: [f32; 2],
    pub properties_width: f32,
    pub timeline_height: f32,
    pub voice_height: f32,
    pub data_override: Option<PathBuf>,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            language: "ja-JP".into(),
            theme: Theme::System,
            ffmpeg: "ffmpeg".into(),
            ffprobe: "ffprobe".into(),
            voicevox_url: "http://127.0.0.1:50021".into(),
            allow_remote_voicevox: false,
            preview_width: 640,
            preview_audio: true,
            audio_device: "default".into(),
            cache_limit_mb: 256,
            export_format: ExportFormat::Mp4,
            window_size: [1200.0, 850.0],
            properties_width: 280.0,
            timeline_height: 240.0,
            voice_height: 64.0,
            data_override: None,
        }
    }
}
impl Settings {
    pub fn load(storage: &Storage) -> (Self, Option<String>) {
        let path = storage.config.join("settings.json");
        if !path.exists() {
            return (Self::default(), None);
        }
        match read_limited(&path, 1024 * 1024) {
            Ok(bytes) => match serde_json::from_str::<Self>(&bytes) {
                Ok(mut settings) => {
                    settings.sanitize();
                    (settings, None)
                }
                Err(e) => (Self::default(), Some(format!("settings-corrupt: {e}"))),
            },
            Err(e) => (Self::default(), Some(format!("settings-corrupt: {e}"))),
        }
    }
    pub fn sanitize(&mut self) {
        self.preview_width = self.preview_width.clamp(160, 1280);
        self.cache_limit_mb = self.cache_limit_mb.clamp(8, 4096);
        for (value, min, max) in [
            (&mut self.properties_width, 220.0, 600.0),
            (&mut self.timeline_height, 140.0, 600.0),
            (&mut self.voice_height, 48.0, 400.0),
            (&mut self.window_size[0], 800.0, 3840.0),
        ] {
            *value = if value.is_finite() {
                value.clamp(min, max)
            } else {
                min
            };
        }
        self.window_size[1] = if self.window_size[1].is_finite() {
            self.window_size[1].clamp(650.0, 2160.0)
        } else {
            850.0
        };
    }
    pub fn save(&self, storage: &Storage) -> Result<()> {
        private_directory(&storage.config)?;
        atomic_write(
            &storage.config.join("settings.json"),
            &serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?,
        )
    }
}
