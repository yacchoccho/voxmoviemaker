use crate::Result;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
};
use uuid::Uuid;

pub type Id = Uuid;
pub const SCHEMA_VERSION: u32 = 4;
pub const MAX_FRAME: u64 = 100_000_000;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct FrameRate {
    pub numerator: u32,
    pub denominator: u32,
}
impl Default for FrameRate {
    fn default() -> Self {
        Self {
            numerator: 30,
            denominator: 1,
        }
    }
}
impl FrameRate {
    pub fn validate(self) -> Result<()> {
        if self.numerator == 0
            || self.denominator == 0
            || self.numerator > 240_000
            || self.denominator > 10_000
        {
            return Err("invalid-frame-rate".into());
        }
        Ok(())
    }
    pub fn seconds(self, frames: u64) -> f64 {
        frames as f64 * self.denominator as f64 / self.numerator as f64
    }
    pub fn frames(self, seconds: f64) -> u64 {
        (seconds * self.numerator as f64 / self.denominator as f64)
            .ceil()
            .clamp(1.0, MAX_FRAME as f64) as u64
    }
    /// Integer conversion, including NTSC rates; never accumulate floating point time.
    pub fn samples(self, frames: u64, sample_rate: u32) -> u64 {
        ((frames as u128 * self.denominator as u128 * sample_rate as u128) / self.numerator as u128)
            as u64
    }
    pub fn ffmpeg(self) -> String {
        format!("{}/{}", self.numerator, self.denominator)
    }
    pub fn timestamp(self, frames: u64) -> String {
        let ns = frames as u128 * self.denominator as u128 * 1_000_000_000 / self.numerator as u128;
        format!("{}.{:09}", ns / 1_000_000_000, ns % 1_000_000_000)
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum MediaKind {
    Video,
    Image,
    Audio,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum AssetLocation {
    External(PathBuf),
    Voice(Id),
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct MediaAsset {
    pub id: Id,
    pub name: String,
    pub location: AssetLocation,
    pub kind: MediaKind,
    pub duration_seconds: f64,
    pub width: u32,
    pub height: u32,
    pub has_audio: bool,
    pub sample_rate: Option<u32>,
    pub frame_rate: Option<FrameRate>,
    pub codec: String,
    pub start_time: f64,
    pub rotation: i32,
    #[serde(default)]
    pub video_stream: Option<u32>,
    #[serde(default)]
    pub audio_stream: Option<u32>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct VoicevoxSettings {
    /// VOICEVOX API's `speaker` query parameter is a style ID.
    pub speaker: u32,
    pub speaker_uuid: String,
    pub speaker_name: String,
    pub style_name: String,
    pub speed: f64,
    pub pitch: f64,
    pub intonation: f64,
    pub volume: f64,
    pub pre_phoneme: f64,
    pub post_phoneme: f64,
    pub pause_length: Option<f64>,
    pub pause_scale: Option<f64>,
    pub sampling_rate: u32,
    pub stereo: bool,
    pub interrogative_upspeak: bool,
}
impl Default for VoicevoxSettings {
    fn default() -> Self {
        Self {
            speaker: 0,
            speaker_uuid: String::new(),
            speaker_name: String::new(),
            style_name: String::new(),
            speed: 1.0,
            pitch: 0.0,
            intonation: 1.0,
            volume: 1.0,
            pre_phoneme: 0.1,
            post_phoneme: 0.1,
            pause_length: None,
            pause_scale: None,
            sampling_rate: 48000,
            stereo: false,
            interrogative_upspeak: true,
        }
    }
}
impl VoicevoxSettings {
    pub fn is_default(&self) -> bool {
        self == &Self::default()
    }
    pub fn validate(&self) -> Result<()> {
        if self
            .pause_length
            .is_some_and(|v| !v.is_finite() || !(0.0..=10.0).contains(&v))
            || self
                .pause_scale
                .is_some_and(|v| !v.is_finite() || !(0.0..=5.0).contains(&v))
            || !(8000..=192000).contains(&self.sampling_rate)
        {
            return Err("invalid-voice-settings".into());
        }
        for (v, min, max) in [
            (self.speed, 0.5, 2.0),
            (self.pitch, -0.15, 0.15),
            (self.intonation, 0.0, 2.0),
            (self.volume, 0.0, 2.0),
            (self.pre_phoneme, 0.0, 1.5),
            (self.post_phoneme, 0.0, 1.5),
        ] {
            if !v.is_finite() || v < min || v > max {
                return Err("invalid-voice-settings".into());
            }
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum ClipSource {
    Text {
        settings: TextSettings,
    },
    Media {
        asset_id: Id,
    },
    Voicevox {
        asset_id: Id,
        text: String,
        #[serde(default, skip_serializing_if = "VoicevoxSettings::is_default")]
        settings: Box<VoicevoxSettings>,
        /// Optional on older projects; new synthesis creates an integrated subtitle.
        #[serde(default)]
        subtitle: Option<TextSettings>,
    },
}
impl ClipSource {
    pub fn asset_id(&self) -> Option<Id> {
        match self {
            Self::Media { asset_id } | Self::Voicevox { asset_id, .. } => Some(*asset_id),
            Self::Text { .. } => None,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Clip {
    pub id: Id,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub character: Option<Box<crate::characters::CharacterBinding>>,
    pub source: ClipSource,
    pub timeline_start: u64,
    pub duration: u64,
    pub source_offset: u64,
    pub volume: f64,
    pub muted: bool,
    #[serde(default)]
    pub effects: ClipEffects,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum TextAlign {
    Left,
    #[default]
    Center,
    Right,
}
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum TextAnchor {
    Top,
    Center,
    #[default]
    Bottom,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct TextSettings {
    pub text: String,
    pub subtitle: bool,
    pub font: PathBuf,
    pub font_index: u32,
    /// Pixel anchor from the upper-left of the project canvas; None preserves legacy layout.
    pub position: Option<[f64; 2]>,
    pub opacity: f64,
    pub size: f32,
    pub color: [u8; 4],
    pub outline: [u8; 4],
    pub outline_width: u32,
    pub align: TextAlign,
    pub anchor: TextAnchor,
}
impl Default for TextSettings {
    fn default() -> Self {
        Self {
            text: String::new(),
            subtitle: true,
            font: PathBuf::new(),
            font_index: 0,
            position: None,
            opacity: 1.0,
            size: 48.0,
            color: [255; 4],
            outline: [0, 0, 0, 255],
            outline_width: 2,
            align: TextAlign::Center,
            anchor: TextAnchor::Bottom,
        }
    }
}
impl TextSettings {
    pub fn validate(&self) -> Result<()> {
        crate::assets::require_utf8_path(&self.font)?;
        if self
            .position
            .is_some_and(|p| p.iter().any(|v| !v.is_finite() || v.abs() > 32768.0))
            || !self.opacity.is_finite()
            || !(0.0..=1.0).contains(&self.opacity)
        {
            return Err("invalid-text-clip".into());
        }
        if self.text.len() > 16_384
            || self.text.chars().count() > 4096
            || !self.size.is_finite()
            || !(8.0..=256.0).contains(&self.size)
            || self.outline_width > 8
            || self.font_index > 64
        {
            return Err("invalid-text-clip".into());
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum TransitionKind {
    #[default]
    None,
    Dissolve,
    SlideLeft,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ClipEffects {
    /// Offset from canvas center in project pixels; scale=1 preserves fit-to-canvas.
    pub x: f64,
    pub y: f64,
    pub scale: f64,
    pub opacity: f64,
    pub fade_in: u64,
    pub fade_out: u64,
    pub audio_fade_in: u64,
    pub audio_fade_out: u64,
    pub transition: TransitionKind,
    pub transition_frames: u64,
}
impl Default for ClipEffects {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            scale: 1.0,
            opacity: 1.0,
            fade_in: 0,
            fade_out: 0,
            audio_fade_in: 0,
            audio_fade_out: 0,
            transition: TransitionKind::None,
            transition_frames: 0,
        }
    }
}
impl ClipEffects {
    pub fn validate(&self, duration: u64) -> Result<()> {
        if !self.x.is_finite()
            || !self.y.is_finite()
            || self.x.abs() > 15360.0
            || self.y.abs() > 8640.0
            || !self.scale.is_finite()
            || !(0.05..=4.0).contains(&self.scale)
            || !self.opacity.is_finite()
            || !(0.0..=1.0).contains(&self.opacity)
            || [
                self.fade_in,
                self.fade_out,
                self.audio_fade_in,
                self.audio_fade_out,
                self.transition_frames,
            ]
            .iter()
            .any(|n| *n > duration)
        {
            return Err("invalid-effects".into());
        }
        Ok(())
    }
    pub fn clamp_duration(&mut self, duration: u64) {
        for value in [
            &mut self.fade_in,
            &mut self.fade_out,
            &mut self.audio_fade_in,
            &mut self.audio_fade_out,
            &mut self.transition_frames,
        ] {
            *value = (*value).min(duration);
        }
    }
    pub fn slide_offset(&self, relative: u64, width: f64) -> f64 {
        if self.transition == TransitionKind::SlideLeft && self.transition_frames > 0 {
            width * (1.0 - relative as f64 / self.transition_frames as f64).clamp(0.0, 1.0)
        } else {
            0.0
        }
    }
    pub fn visual_fade_in(&self) -> u64 {
        self.fade_in
            .max(if self.transition == TransitionKind::Dissolve {
                self.transition_frames
            } else {
                0
            })
    }
    pub fn visual_alpha(&self, relative: u64, duration: u64) -> f64 {
        let fade = |n: u64, d: u64| {
            if d == 0 {
                1.0
            } else {
                (n as f64 / d as f64).clamp(0.0, 1.0)
            }
        };
        self.opacity
            * fade(relative, self.visual_fade_in())
            * fade(duration.saturating_sub(relative), self.fade_out)
    }
}
impl Clip {
    pub fn end(&self) -> u64 {
        self.timeline_start.saturating_add(self.duration)
    }
    pub fn active(&self, frame: u64) -> bool {
        self.timeline_start <= frame && frame < self.end()
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Track {
    pub id: Id,
    pub name: String,
    pub enabled: bool,
    pub video_visible: bool,
    pub muted: bool,
    pub locked: bool,
    pub clips: Vec<Clip>,
}
impl Track {
    pub fn new(number: usize) -> Self {
        Self {
            id: Id::new_v4(),
            name: format!("Layer {number}"),
            enabled: true,
            video_visible: true,
            muted: false,
            locked: false,
            clips: Vec::new(),
        }
    }
    pub fn fits(&self, start: u64, duration: u64, except: Option<Id>) -> bool {
        duration > 0
            && start.checked_add(duration).is_some_and(|end| {
                end <= MAX_FRAME
                    && self.clips.iter().all(|c| {
                        Some(c.id) == except || start >= c.end() || end <= c.timeline_start
                    })
            })
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Project {
    pub schema_version: u32,
    pub project_uuid: Id,
    pub width: u32,
    pub height: u32,
    pub frame_rate: FrameRate,
    pub assets: Vec<MediaAsset>,
    /// Authoritative back-to-front order. IDs do not determine stacking.
    pub tracks: Vec<Track>,
}
impl Default for Project {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            project_uuid: Id::new_v4(),
            width: 1280,
            height: 720,
            frame_rate: FrameRate::default(),
            assets: Vec::new(),
            tracks: (1..=5).map(Track::new).collect(),
        }
    }
}
impl Project {
    pub fn duration(&self) -> u64 {
        self.tracks
            .iter()
            .flat_map(|t| &t.clips)
            .map(Clip::end)
            .max()
            .unwrap_or(1)
    }
    pub fn asset(&self, id: Id) -> Result<&MediaAsset> {
        self.assets
            .iter()
            .find(|a| a.id == id)
            .ok_or_else(|| "missing-asset".into())
    }
    pub fn clip(&self, id: Id) -> Option<&Clip> {
        self.tracks
            .iter()
            .flat_map(|t| &t.clips)
            .find(|c| c.id == id)
    }
    pub fn clip_mut(&mut self, id: Id) -> Option<&mut Clip> {
        self.tracks
            .iter_mut()
            .flat_map(|t| &mut t.clips)
            .find(|c| c.id == id)
    }
    pub fn track_of(&self, id: Id) -> Option<usize> {
        self.tracks
            .iter()
            .position(|t| t.clips.iter().any(|c| c.id == id))
    }
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != SCHEMA_VERSION {
            return Err("unsupported-schema".into());
        }
        self.frame_rate.validate()?;
        if self.width < 2
            || self.height < 2
            || self.width > 7680
            || self.height > 4320
            || !self.width.is_multiple_of(2)
            || !self.height.is_multiple_of(2)
            || self.tracks.is_empty()
            || self.tracks.len() > 256
            || self.assets.len() > 10_000
        {
            return Err("invalid-project".into());
        }
        let mut ids = HashSet::new();
        let mut assets = HashMap::new();
        for asset in &self.assets {
            if let AssetLocation::External(path) = &asset.location {
                crate::assets::require_utf8_path(path)?;
            }
            assets.insert(asset.id, asset);
            if !ids.insert(asset.id)
                || !asset.duration_seconds.is_finite()
                || asset.duration_seconds < 0.0
                || !asset.start_time.is_finite()
            {
                return Err("invalid-asset".into());
            }
        }
        for track in &self.tracks {
            if !ids.insert(track.id) || track.clips.len() > 10_000 {
                return Err("invalid-track".into());
            }
            let mut clips: Vec<_> = track.clips.iter().collect();
            clips.sort_unstable_by_key(|c| c.timeline_start);
            let mut previous_end = 0;
            for clip in clips {
                let end = clip.timeline_start.checked_add(clip.duration);
                if !ids.insert(clip.id)
                    || clip.duration == 0
                    || clip.timeline_start < previous_end
                    || end.is_none_or(|end| end > MAX_FRAME)
                    || clip
                        .source_offset
                        .checked_add(clip.duration)
                        .is_none_or(|x| x > MAX_FRAME)
                    || !clip.volume.is_finite()
                    || !(0.0..=4.0).contains(&clip.volume)
                {
                    return Err("collision-or-invalid-clip".into());
                }
                previous_end = clip.end();
                clip.effects.validate(clip.duration)?;
                if let Some(binding) = &clip.character {
                    if matches!(clip.source, ClipSource::Media { .. }) {
                        return Err("invalid-character".into());
                    }
                    binding.validate()?;
                }
                if let Some(asset_id) = clip.source.asset_id() {
                    let asset = assets.get(&asset_id).ok_or("missing-asset")?;
                    if asset.kind != MediaKind::Image
                        && clip.source_offset + clip.duration
                            > self.frame_rate.frames(asset.duration_seconds)
                    {
                        return Err("trim-outside-source".into());
                    }
                }
                match &clip.source {
                    ClipSource::Text { settings } => settings.validate()?,
                    ClipSource::Voicevox {
                        subtitle: Some(settings),
                        ..
                    } => settings.validate()?,
                    _ => {}
                }
                if let ClipSource::Voicevox { settings, text, .. } = &clip.source {
                    settings.validate()?;
                    if text.len() > 100_000 {
                        return Err("text-too-long".into());
                    }
                }
            }
        }
        Ok(())
    }
}
