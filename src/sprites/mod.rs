//! Shared sprite definitions live in XDG config; projects retain only their UUID.
pub mod psd;
pub mod render;
mod storage;
pub use storage::SpriteStore;

/// Bump when sprite rendering changes so old previews/proxies are not reused.
pub(crate) const RENDER_CACHE_VERSION: u32 = 2;

use crate::{
    Result,
    project::{FrameRate, Id, MAX_FRAME},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    path::PathBuf,
};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AnimationFrame {
    pub layers: Vec<String>,
    pub duration_ms: u32,
}
impl Default for AnimationFrame {
    fn default() -> Self {
        Self {
            layers: Vec::new(),
            duration_ms: 100,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Animation {
    pub enabled: bool,
    pub frames: Vec<AnimationFrame>,
    pub idle_layers: Vec<String>,
    /// Integer thousandths avoid accumulating floating point animation time.
    pub speed_milli: u32,
    pub repeat: bool,
}
impl Default for Animation {
    fn default() -> Self {
        Self {
            enabled: false,
            frames: vec![AnimationFrame::default(); 3],
            idle_layers: Vec::new(),
            speed_milli: 1000,
            repeat: true,
        }
    }
}
impl Animation {
    pub fn next_change(&self, frame: u64, fps: FrameRate, interval_ms: Option<u32>) -> u64 {
        if !self.enabled || self.frames.is_empty() {
            return u64::MAX;
        }
        let unit = fps.numerator as u128 * 1000;
        let per_frame = fps.denominator as u128 * 1000 * self.speed_milli as u128;
        let ticks = frame as u128 * per_frame;
        let total = self
            .frames
            .iter()
            .map(|f| f.duration_ms as u128)
            .sum::<u128>()
            * unit;
        if total == 0 {
            return u64::MAX;
        }
        let idle = interval_ms.unwrap_or(0) as u128 * unit;
        let period = idle + total;
        if interval_ms.is_none() && !self.repeat && ticks >= total {
            return u64::MAX;
        }
        let cycle = if interval_ms.is_some() || self.repeat {
            ticks / period
        } else {
            0
        };
        let phase = ticks - cycle * period;
        let mut end = idle;
        if phase >= idle {
            for step in &self.frames {
                end += step.duration_ms as u128 * unit;
                if phase < end {
                    break;
                }
            }
        }
        ((cycle * period + end).div_ceil(per_frame)).min(u64::MAX as u128) as u64
    }
    pub fn validate(&self) -> Result<()> {
        if self.frames.is_empty()
            || self.frames.len() > 128
            || !(100..=10000).contains(&self.speed_milli)
            || self
                .frames
                .iter()
                .any(|f| !(1..=60000).contains(&f.duration_ms))
        {
            return Err("sprite-invalid".into());
        }
        for layers in
            std::iter::once(&self.idle_layers).chain(self.frames.iter().map(|f| &f.layers))
        {
            if layers.len() > 2048 || layers.iter().any(|k| k.len() > 1024) {
                return Err("sprite-invalid".into());
            }
        }
        Ok(())
    }
    pub fn keys(&self) -> impl Iterator<Item = &String> {
        self.idle_layers
            .iter()
            .chain(self.frames.iter().flat_map(|f| &f.layers))
    }
    /// Evaluate an absolute rational timeline position, never an independent clock.
    pub fn layers_at(
        &self,
        frames: u64,
        fps: FrameRate,
        active: bool,
        interval_ms: Option<u32>,
    ) -> &[String] {
        if !self.enabled || !active || self.frames.is_empty() {
            return &self.idle_layers;
        }
        let ticks = frames as u128 * fps.denominator as u128 * 1000 * self.speed_milli as u128;
        let divisor = fps.numerator as u128 * 1000;
        let total = self
            .frames
            .iter()
            .map(|f| f.duration_ms as u128)
            .sum::<u128>()
            * divisor;
        if total == 0 {
            return &self.idle_layers;
        }
        let phase = if let Some(interval) = interval_ms {
            let idle = interval as u128 * divisor;
            let phase = ticks % (idle + total);
            if phase < idle {
                return &self.idle_layers;
            }
            phase - idle
        } else if self.repeat {
            ticks % total
        } else {
            ticks.min(total - 1)
        };
        let mut end = 0;
        for step in &self.frames {
            end += step.duration_ms as u128 * divisor;
            if phase < end {
                return &step.layers;
            }
        }
        &self.idle_layers
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Sprite {
    pub id: Id,
    pub name: String,
    pub psd: PathBuf,
    pub character_id: Option<Id>,
    /// Only visibility overrides; missing keys use the original PSD flags.
    pub visibility: BTreeMap<String, bool>,
    pub mouth: Animation,
    pub blink: Animation,
    pub blink_interval_ms: u32,
}
impl Sprite {
    pub fn new(name: String) -> Self {
        Self {
            id: Id::new_v4(),
            name,
            psd: PathBuf::new(),
            character_id: None,
            visibility: BTreeMap::new(),
            mouth: Animation::default(),
            blink: Animation::default(),
            blink_interval_ms: 3000,
        }
    }
    pub fn validate(&self) -> Result<()> {
        crate::assets::require_utf8_path(&self.psd)?;
        if self.id.is_nil()
            || self.name.trim().is_empty()
            || self.name.len() > 1024
            || (!self.psd.as_os_str().is_empty() && !self.psd.is_absolute())
            || self.character_id.is_some_and(|id| id.is_nil())
            || self.visibility.len() > 2048
            || self.visibility.keys().any(|k| k.len() > 1024)
            || !(100..=600000).contains(&self.blink_interval_ms)
        {
            return Err("sprite-invalid".into());
        }
        self.mouth.validate()?;
        self.blink.validate()
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SpriteLibrary {
    pub schema_version: u32,
    pub sprites: Vec<Sprite>,
}
impl Default for SpriteLibrary {
    fn default() -> Self {
        Self {
            schema_version: 1,
            sprites: Vec::new(),
        }
    }
}
impl SpriteLibrary {
    pub fn get(&self, id: Id) -> Option<&Sprite> {
        self.sprites.iter().find(|s| s.id == id)
    }
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != 1 || self.sprites.len() > 1024 {
            return Err("sprite-invalid".into());
        }
        let mut ids = HashSet::new();
        for sprite in &self.sprites {
            if !ids.insert(sprite.id) {
                return Err("sprite-invalid".into());
            }
            sprite.validate()?;
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize)]
pub struct SpeechSpan {
    pub start: u64,
    pub end: u64,
}
/// Union same-character intervals: overlapping voices do not race or restart the mouth.
pub fn speech_spans(project: &crate::project::Project, character: Option<Id>) -> Vec<SpeechSpan> {
    let Some(character) = character else {
        return Vec::new();
    };
    let mut spans: Vec<_> = project
        .tracks
        .iter()
        .filter(|t| t.enabled && !t.muted)
        .flat_map(|t| &t.clips)
        .filter(|c| {
            !c.muted
                && matches!(c.source, crate::project::ClipSource::Voicevox { .. })
                && c.character
                    .as_ref()
                    .is_some_and(|b| b.character_id == character)
        })
        .map(|c| SpeechSpan {
            start: c.timeline_start,
            end: c.end().min(MAX_FRAME),
        })
        .collect();
    spans.sort_by_key(|s| s.start);
    let mut merged: Vec<SpeechSpan> = Vec::new();
    for span in spans {
        if let Some(last) = merged.last_mut().filter(|s| s.end >= span.start) {
            last.end = last.end.max(span.end);
        } else {
            merged.push(span);
        }
    }
    merged
}
#[derive(Clone, Debug, Serialize)]
pub struct SpriteRender {
    pub source_revision: Option<[u64; 7]>,
    pub settings: Sprite,
    pub speech: Vec<SpeechSpan>,
}
impl SpriteRender {
    pub fn next_change(&self, local: u64, timeline: u64, fps: FrameRate) -> u64 {
        let mut delta = self
            .settings
            .blink
            .next_change(local, fps, Some(self.settings.blink_interval_ms))
            .saturating_sub(local);
        if self.settings.mouth.enabled {
            if let Some(span) = self
                .speech
                .iter()
                .find(|s| s.start <= timeline && timeline < s.end)
            {
                delta = delta.min(span.end - timeline).min(
                    self.settings
                        .mouth
                        .next_change(timeline - span.start, fps, None)
                        .saturating_sub(timeline - span.start),
                );
            } else if let Some(span) = self.speech.iter().find(|s| s.start > timeline) {
                delta = delta.min(span.start - timeline);
            }
        }
        delta.max(1)
    }
    pub fn selected_layers(
        &self,
        local: u64,
        timeline: u64,
        fps: FrameRate,
    ) -> (BTreeSet<String>, BTreeSet<String>) {
        let speech = self
            .speech
            .iter()
            .find(|s| s.start <= timeline && timeline < s.end);
        let mouth = self.settings.mouth.layers_at(
            speech.map_or(0, |s| timeline - s.start),
            fps,
            speech.is_some(),
            None,
        );
        let eyes =
            self.settings
                .blink
                .layers_at(local, fps, true, Some(self.settings.blink_interval_ms));
        (
            mouth.iter().cloned().collect(),
            eyes.iter().cloned().collect(),
        )
    }
}
