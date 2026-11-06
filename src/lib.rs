pub mod app;
pub mod assets;
pub mod characters;
pub mod i18n;
pub mod media;
pub mod project;
pub mod settings;
pub mod sprites;
pub mod tasks;
pub mod ui;
pub mod voicevox;

pub type Result<T> = std::result::Result<T, String>;
