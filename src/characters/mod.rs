//! Project-independent character defaults and sparse per-clip overrides.
mod storage;
use crate::{Result, project::*};
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, path::PathBuf};
pub use storage::CharacterStore;

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub enum Engine {
    #[default]
    Voicevox,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Character {
    pub id: Id,
    pub name: String,
    #[serde(default)]
    pub engine: Engine,
    pub voice: VoicevoxSettings,
    pub subtitle: TextSettings,
}
impl Character {
    pub fn new(name: String, width: u32, height: u32) -> Self {
        Self {
            id: Id::new_v4(),
            name,
            engine: Engine::Voicevox,
            voice: VoicevoxSettings::default(),
            subtitle: TextSettings {
                position: Some([width as f64 / 2.0, height as f64 - 32.0]),
                ..Default::default()
            },
        }
    }
    pub fn validate(&self) -> Result<()> {
        if self.id.is_nil()
            || self.name.trim().is_empty()
            || self.name.len() > 1024
            || !self.subtitle.text.is_empty()
        {
            return Err("invalid-character".into());
        }
        self.voice.validate()?;
        self.subtitle.validate()
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CharacterLibrary {
    pub schema_version: u32,
    pub characters: Vec<Character>,
}
impl Default for CharacterLibrary {
    fn default() -> Self {
        Self {
            schema_version: 1,
            characters: Vec::new(),
        }
    }
}
impl CharacterLibrary {
    pub fn get(&self, id: Id) -> Option<&Character> {
        self.characters.iter().find(|c| c.id == id)
    }
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != 1 || self.characters.len() > 1024 {
            return Err("invalid-character-library".into());
        }
        let mut ids = HashSet::new();
        for character in &self.characters {
            character.validate()?;
            if !ids.insert(character.id) {
                return Err("invalid-character-library".into());
            }
        }
        Ok(())
    }
}

/// This pair is atomic: the API's `speaker` argument is a style ID, never a UUID.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct VoiceIdentity {
    pub speaker_uuid: String,
    pub style_id: u32,
    pub speaker_name: String,
    pub style_name: String,
}
impl VoiceIdentity {
    pub fn apply(&self, settings: &mut VoicevoxSettings) {
        settings.speaker_uuid = self.speaker_uuid.clone();
        settings.speaker = self.style_id;
        settings.speaker_name = self.speaker_name.clone();
        settings.style_name = self.style_name.clone();
    }
    pub fn from_settings(settings: &VoicevoxSettings) -> Self {
        Self {
            speaker_uuid: settings.speaker_uuid.clone(),
            style_id: settings.speaker,
            speaker_name: settings.speaker_name.clone(),
            style_name: settings.style_name.clone(),
        }
    }
}
/// A present wrapper with value=None explicitly requests the ENGINE's default.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Nullable<T> {
    pub value: Option<T>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct VoiceOverrides {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identity: Option<VoiceIdentity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speed: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pitch: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub intonation: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub volume: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pre_phoneme: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub post_phoneme: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pause_length: Option<Nullable<f64>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pause_scale: Option<Nullable<f64>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sampling_rate: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stereo: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interrogative_upspeak: Option<bool>,
}
impl VoiceOverrides {
    pub fn apply(&self, base: &VoicevoxSettings) -> VoicevoxSettings {
        let mut value = base.clone();
        if let Some(identity) = &self.identity {
            identity.apply(&mut value);
        }
        macro_rules! apply { ($($field:ident),*) => { $(if let Some(v) = self.$field { value.$field = v; })* }; }
        apply!(
            speed,
            pitch,
            intonation,
            volume,
            pre_phoneme,
            post_phoneme,
            sampling_rate,
            stereo,
            interrogative_upspeak
        );
        if let Some(v) = &self.pause_length {
            value.pause_length = v.value;
        }
        if let Some(v) = &self.pause_scale {
            value.pause_scale = v.value;
        }
        value
    }
    pub fn all(value: &VoicevoxSettings) -> Self {
        Self {
            identity: Some(VoiceIdentity::from_settings(value)),
            speed: Some(value.speed),
            pitch: Some(value.pitch),
            intonation: Some(value.intonation),
            volume: Some(value.volume),
            pre_phoneme: Some(value.pre_phoneme),
            post_phoneme: Some(value.post_phoneme),
            pause_length: Some(Nullable {
                value: value.pause_length,
            }),
            pause_scale: Some(Nullable {
                value: value.pause_scale,
            }),
            sampling_rate: Some(value.sampling_rate),
            stereo: Some(value.stereo),
            interrogative_upspeak: Some(value.interrogative_upspeak),
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct SubtitleOverrides {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub x: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub y: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_index: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<[u8; 4]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outline: Option<[u8; 4]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outline_width: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub opacity: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub align: Option<TextAlign>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anchor: Option<TextAnchor>,
}
impl SubtitleOverrides {
    pub fn apply(&self, base: &TextSettings) -> TextSettings {
        let mut value = base.clone();
        macro_rules! apply { ($($field:ident),*) => { $(if let Some(v) = &self.$field { value.$field = v.clone(); })* }; }
        apply!(
            font,
            font_index,
            size,
            color,
            outline,
            outline_width,
            opacity,
            align,
            anchor
        );
        if self.x.is_some() || self.y.is_some() {
            let p = value.position.get_or_insert([640.0, 660.0]);
            if let Some(x) = self.x {
                p[0] = x;
            }
            if let Some(y) = self.y {
                p[1] = y;
            }
        }
        value
    }
    pub fn all(value: &TextSettings) -> Self {
        let [x, y] = value.position.unwrap_or([640.0, 660.0]);
        Self {
            x: Some(x),
            y: Some(y),
            font: Some(value.font.clone()),
            font_index: Some(value.font_index),
            size: Some(value.size),
            color: Some(value.color),
            outline: Some(value.outline),
            outline_width: Some(value.outline_width),
            opacity: Some(value.opacity),
            align: Some(value.align),
            anchor: Some(value.anchor),
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CharacterBinding {
    pub character_id: Id,
    #[serde(default)]
    pub voice: VoiceOverrides,
    #[serde(default)]
    pub subtitle: SubtitleOverrides,
    #[serde(default = "enabled")]
    pub subtitle_enabled: bool,
}
fn enabled() -> bool {
    true
}
impl CharacterBinding {
    pub fn new(character_id: Id) -> Self {
        Self {
            character_id,
            voice: Default::default(),
            subtitle: Default::default(),
            subtitle_enabled: true,
        }
    }
    pub fn voice(&self, library: &CharacterLibrary) -> Result<VoicevoxSettings> {
        let character = library.get(self.character_id).ok_or("character-missing")?;
        let settings = self.voice.apply(&character.voice);
        settings.validate()?;
        Ok(settings)
    }
    pub fn text(&self, library: &CharacterLibrary, text: &str) -> TextSettings {
        let default = TextSettings {
            position: Some([640.0, 660.0]),
            ..Default::default()
        };
        let base = library
            .get(self.character_id)
            .map_or(&default, |c| &c.subtitle);
        let mut settings = self.subtitle.apply(base);
        settings.text = text.into();
        settings
    }
    pub fn validate(&self) -> Result<()> {
        if self.character_id.is_nil() {
            return Err("invalid-character".into());
        }
        self.voice.apply(&VoicevoxSettings::default()).validate()?;
        self.subtitle.apply(&TextSettings::default()).validate()
    }
}
