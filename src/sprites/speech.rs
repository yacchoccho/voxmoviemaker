//! Bounded WAV envelopes and deterministic speech gates. Called only by render workers.
use super::{MouthDetection, SpeechSpan, SpriteRender, psd};
use crate::{
    Result,
    assets::private_directory,
    project::persistence::{atomic_write, read_limited},
    project::{FrameRate, MAX_FRAME},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{VecDeque, hash_map::DefaultHasher},
    fs,
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
};

const WINDOW_MS: u64 = 10;
const MAX_WINDOWS: usize = 360_000; // One hour, <= 1.44 MB of RMS values per source.
#[derive(Clone, Debug, Serialize)]
pub struct VoiceSource {
    pub path: PathBuf,
    pub revision: Option<[u64; 7]>,
    pub start: u64,
    pub offset: u64,
    pub duration: u64,
    pub volume: f64,
}
#[derive(Serialize, Deserialize)]
struct Envelope {
    version: u32,
    path: PathBuf,
    revision: [u64; 7],
    levels: Vec<f32>,
    duration_us: u64,
}
type MemoryCache = Mutex<VecDeque<Arc<Envelope>>>;
type GateCache = Mutex<VecDeque<(Vec<u8>, Vec<SpeechSpan>)>>;
static GATES: OnceLock<GateCache> = OnceLock::new();
static ENVELOPES: OnceLock<MemoryCache> = OnceLock::new();

fn envelope(
    source: &VoiceSource,
    directory: &Path,
    cancelled: &dyn Fn() -> bool,
) -> Result<Arc<Envelope>> {
    if cancelled() {
        return Err("cancelled".into());
    }
    let revision = psd::revision(&source.path).ok_or("sprite-voice-missing")?;
    if source.revision != Some(revision) {
        return Err("sprite-voice-changed".into());
    }
    let memory = ENVELOPES.get_or_init(Default::default);
    if let Some(found) = memory
        .lock()
        .map_err(|_| "sprite-cache-error")?
        .iter()
        .find(|e| e.path == source.path && e.revision == revision)
        .cloned()
    {
        return Ok(found);
    }
    let mut hash = DefaultHasher::new();
    (1u32, &source.path, revision).hash(&mut hash);
    let path = directory.join(format!("vmm-speech-{:016x}.json", hash.finish()));
    let cached = read_limited(&path, 16 * 1024 * 1024)
        .ok()
        .and_then(|text| serde_json::from_str::<Envelope>(&text).ok())
        .filter(|e| {
            e.version == 1
                && e.path == source.path
                && e.revision == revision
                && e.levels.len() <= MAX_WINDOWS
                && e.duration_us <= 3_600_000_000
                && e.levels.len() as u64 == e.duration_us.div_ceil(WINDOW_MS * 1000)
                && e.levels
                    .iter()
                    .all(|v| v.is_finite() && (0.0..=1.0).contains(v))
        });
    let result = if let Some(cached) = cached {
        cached
    } else {
        let decoded = decode(&source.path, revision, cancelled)?;
        if psd::revision(&source.path) != Some(revision) {
            return Err("sprite-voice-changed".into());
        }
        private_directory(directory)?;
        prune(directory);
        atomic_write(
            &path,
            &serde_json::to_vec(&decoded).map_err(|e| e.to_string())?,
        )?;
        decoded
    };
    let result = Arc::new(result);
    let mut cache = memory.lock().map_err(|_| "sprite-cache-error")?;
    cache.retain(|e| e.path != source.path);
    while cache.len() >= 8 {
        cache.pop_front();
    }
    cache.push_back(result.clone());
    Ok(result)
}
fn decode(path: &Path, revision: [u64; 7], cancelled: &dyn Fn() -> bool) -> Result<Envelope> {
    let mut reader =
        hound::WavReader::open(path).map_err(|e| format!("sprite-voice-invalid: {e}"))?;
    let spec = reader.spec();
    if spec.channels == 0
        || spec.channels > 8
        || !(1000..=192000).contains(&spec.sample_rate)
        || spec.bits_per_sample == 0
        || spec.bits_per_sample > 32
    {
        return Err("sprite-voice-invalid".into());
    }
    let frames = reader.duration() as u64;
    if frames > spec.sample_rate as u64 * 3600 {
        return Err("sprite-voice-too-long".into());
    }
    let channels = spec.channels as u64;
    let rate = spec.sample_rate as u64;
    let duration_us = (frames * 1_000_000).div_ceil(rate);
    let mut levels = vec![0.0; duration_us.div_ceil(10_000) as usize];
    let mut bin = 0usize;
    let mut sum = 0.0f64;
    let mut count = 0u64;
    let mut samples_seen = 0u64;
    let mut consume = |value: f64| -> Result<()> {
        let frame = samples_seen / channels;
        let next = (frame * 1000 / (rate * WINDOW_MS)) as usize;
        if samples_seen.is_multiple_of(4096) && cancelled() {
            return Err("cancelled".into());
        }
        if next >= levels.len() || !value.is_finite() {
            return Err("sprite-voice-invalid".into());
        }
        if next != bin {
            levels[bin] = (sum / count as f64).sqrt() as f32;
            sum = 0.0;
            count = 0;
            bin = next;
        }
        sum += value.clamp(-1.0, 1.0).powi(2);
        count += 1;
        samples_seen += 1;
        Ok(())
    };
    match spec.sample_format {
        hound::SampleFormat::Float => {
            for sample in reader.samples::<f32>() {
                consume(sample.map_err(|e| format!("sprite-voice-invalid: {e}"))? as f64)?;
            }
        }
        hound::SampleFormat::Int => {
            let scale = 2f64.powi(spec.bits_per_sample as i32 - 1);
            for sample in reader.samples::<i32>() {
                consume(sample.map_err(|e| format!("sprite-voice-invalid: {e}"))? as f64 / scale)?;
            }
        }
    }
    if samples_seen != frames * channels {
        return Err("sprite-voice-invalid".into());
    }
    if count > 0 {
        levels[bin] = (sum / count as f64).sqrt() as f32;
    }
    Ok(Envelope {
        version: 1,
        path: path.to_path_buf(),
        revision,
        levels,
        duration_us,
    })
}
fn prune(directory: &Path) {
    let mut entries: Vec<_> = fs::read_dir(directory)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| {
            e.file_name().to_string_lossy().starts_with("vmm-speech-")
                && e.path().extension().is_some_and(|e| e == "json")
        })
        .filter_map(|e| {
            e.metadata()
                .ok()
                .filter(|m| m.is_file())
                .map(|m| (m.modified().ok(), m.len(), e.path()))
        })
        .collect();
    entries.sort_by_key(|e| e.0);
    let mut total: u64 = entries.iter().map(|e| e.1).sum();
    for (_, len, path) in entries {
        if total <= 48 * 1024 * 1024 {
            break;
        }
        if fs::remove_file(path).is_ok() {
            total = total.saturating_sub(len);
        }
    }
}

/// Analyze each trimmed clip independently, then union intervals. Overlapping voices
/// open the same mouth if any one is audible; they never restart a looping sequence.
pub fn resolve(
    sprite: &SpriteRender,
    fps: FrameRate,
    directory: &Path,
    cancelled: &dyn Fn() -> bool,
) -> Result<SpriteRender> {
    let mut resolved = sprite.clone();
    resolved.speech.clear();
    if !sprite.settings.mouth.enabled {
        return Ok(resolved);
    }
    for source in &sprite.voices {
        if source.volume <= 0.0 {
            continue;
        }
        let envelope = envelope(source, directory, cancelled)?;
        let key = serde_json::to_vec(&(source, fps, &sprite.settings.mouth_detection))
            .map_err(|e| e.to_string())?;
        let gates = GATES.get_or_init(Default::default);
        let cached = gates
            .lock()
            .map_err(|_| "sprite-cache-error")?
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, spans)| spans.clone());
        let spans = if let Some(spans) = cached {
            spans
        } else {
            let spans = gate(
                &envelope,
                source,
                fps,
                &sprite.settings.mouth_detection,
                cancelled,
            )?;
            let mut cache = gates.lock().map_err(|_| "sprite-cache-error")?;
            while cache.len() >= 8 {
                cache.pop_front();
            }
            cache.push_back((key, spans.clone()));
            spans
        };
        resolved.speech.extend(spans);
    }
    resolved.speech.sort_by_key(|s| s.start);
    let mut merged: Vec<SpeechSpan> = Vec::new();
    for span in resolved.speech {
        if let Some(last) = merged.last_mut().filter(|s| s.end >= span.start) {
            last.end = last.end.max(span.end);
        } else {
            merged.push(span);
        }
    }
    resolved.speech = merged;
    Ok(resolved)
}
fn gate(
    envelope: &Envelope,
    source: &VoiceSource,
    fps: FrameRate,
    config: &MouthDetection,
    cancelled: &dyn Fn() -> bool,
) -> Result<Vec<SpeechSpan>> {
    // Units are microseconds * fps numerator; all clip and window boundaries are exact.
    let n = fps.numerator as u128;
    let frame_unit = fps.denominator as u128 * 1_000_000;
    let begin = source.offset as u128 * frame_unit;
    let end = ((source.offset as u128 + source.duration as u128) * frame_unit)
        .min(envelope.duration_us as u128 * n);
    let attack = config.attack_ms as u128 * 1000 * n;
    let release = config.release_ms as u128 * 1000 * n;
    let threshold = 10f64.powf(config.threshold_db as f64 / 20.0);
    let mut candidate = None;
    let mut opened = None;
    let mut silence = None;
    let mut spans = Vec::new();
    let mut push = |a: u128, b: u128| {
        let start = source
            .start
            .saturating_add((a.saturating_sub(begin)).div_ceil(frame_unit) as u64);
        let end = source
            .start
            .saturating_add((b.saturating_sub(begin)).div_ceil(frame_unit) as u64)
            .min(source.start.saturating_add(source.duration))
            .min(MAX_FRAME);
        if start < end {
            spans.push(SpeechSpan { start, end });
        }
    };
    for (i, level) in envelope.levels.iter().enumerate() {
        if i.is_multiple_of(4096) && cancelled() {
            return Err("cancelled".into());
        }
        let a = (i as u128 * 10_000 * n).max(begin);
        let b = ((i as u128 + 1) * 10_000 * n).min(end);
        if a >= b {
            continue;
        }
        if *level as f64 * source.volume >= threshold {
            silence = None;
            if opened.is_none() {
                let since = *candidate.get_or_insert(a);
                if b > since + attack {
                    opened = Some(since + attack);
                }
            }
        } else {
            candidate = None;
            if let Some(start) = opened {
                let since = *silence.get_or_insert(a);
                if b >= since + release {
                    push(start, since + release);
                    opened = None;
                    silence = None;
                }
            }
        }
    }
    if let Some(start) = opened {
        push(start, end);
    }
    Ok(spans)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sprites::Sprite;
    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!("vmm-speech-test-{}", uuid::Uuid::new_v4()));
            fs::create_dir(&p).unwrap();
            Self(p)
        }
        fn wav(&self, floating: bool) -> VoiceSource {
            let path = self.0.join(if floating { "float.wav" } else { "声.wav" });
            let mut writer = hound::WavWriter::create(
                &path,
                hound::WavSpec {
                    channels: 2,
                    sample_rate: 48000,
                    bits_per_sample: if floating { 32 } else { 16 },
                    sample_format: if floating {
                        hound::SampleFormat::Float
                    } else {
                        hound::SampleFormat::Int
                    },
                },
            )
            .unwrap();
            // 0..100ms silence, 100..300ms voice, 10ms gap, voice until 500ms,
            // silence, a 10ms click (too short to open), then low noise.
            for i in 0..48000 {
                let ms = i / 48;
                let v: f32 = if (100..300).contains(&ms)
                    || (310..500).contains(&ms)
                    || (700..710).contains(&ms)
                {
                    0.25
                } else if ms >= 800 {
                    0.0001
                } else {
                    0.0
                };
                for value in [v, -v] {
                    // Opposite stereo phase must not cancel detection.
                    if floating {
                        writer.write_sample(value).unwrap();
                    } else {
                        writer.write_sample((value * 32768.0) as i16).unwrap();
                    }
                }
            }
            writer.finalize().unwrap();
            VoiceSource {
                revision: psd::revision(&path),
                path,
                start: 0,
                offset: 0,
                duration: 1000,
                volume: 1.0,
            }
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn render(source: VoiceSource) -> SpriteRender {
        let mut settings = Sprite::new("test".into());
        settings.mouth.enabled = true;
        SpriteRender {
            settings,
            voices: vec![source],
            speech: Vec::new(),
            source_revision: None,
        }
    }
    fn ms_fps() -> FrameRate {
        FrameRate {
            numerator: 1000,
            denominator: 1,
        }
    }
    fn pairs(r: &SpriteRender) -> Vec<(u64, u64)> {
        r.speech.iter().map(|s| (s.start, s.end)).collect()
    }
    #[test]
    fn audible_runs_hold_open_across_short_gaps_and_ignore_clicks_and_noise() {
        let temp = Temp::new();
        for float in [false, true] {
            let sprite = render(temp.wav(float));
            let resolved = resolve(&sprite, ms_fps(), &temp.0.join("cache"), &|| false).unwrap();
            assert_eq!(pairs(&resolved), [(120, 580)]);
            let muted = render(VoiceSource {
                volume: 0.0,
                ..sprite.voices[0].clone()
            });
            assert!(
                resolve(&muted, ms_fps(), &temp.0.join("cache"), &|| false)
                    .unwrap()
                    .speech
                    .is_empty()
            );
            let mut quiet = sprite.clone();
            quiet.voices[0].volume = 0.001;
            assert!(
                resolve(&quiet, ms_fps(), &temp.0.join("cache"), &|| false)
                    .unwrap()
                    .speech
                    .is_empty()
            );
            let mut sensitive = sprite.clone();
            sensitive.settings.mouth_detection.threshold_db = -6.0;
            assert!(
                resolve(&sensitive, ms_fps(), &temp.0.join("cache"), &|| false)
                    .unwrap()
                    .speech
                    .is_empty()
            );
            let mut immediate = sprite.clone();
            immediate.settings.mouth_detection.attack_ms = 0;
            immediate.settings.mouth_detection.release_ms = 0;
            assert_eq!(
                pairs(&resolve(&immediate, ms_fps(), &temp.0.join("cache"), &|| false).unwrap()),
                [(100, 300), (310, 500), (700, 710)]
            );
        }
    }
    #[test]
    fn trim_move_overlap_and_ntsc_seeks_have_deterministic_boundaries() {
        let temp = Temp::new();
        let mut sprite = render(temp.wav(false));
        sprite.voices[0].offset = 200;
        sprite.voices[0].start = 2000;
        sprite.voices[0].duration = 250;
        let resolved = resolve(&sprite, ms_fps(), &temp.0.join("cache"), &|| false).unwrap();
        assert_eq!(pairs(&resolved), [(2020, 2250)]);
        sprite.voices.push(VoiceSource {
            start: 2200,
            ..sprite.voices[0].clone()
        });
        assert_eq!(
            pairs(&resolve(&sprite, ms_fps(), &temp.0.join("cache"), &|| false).unwrap()),
            [(2020, 2450)]
        );
        sprite.voices.truncate(1);
        sprite.voices[0].start = 0;
        sprite.voices[0].offset = 0;
        sprite.voices[0].duration = 100;
        for numerator in [24000, 30000] {
            let fps = FrameRate {
                numerator,
                denominator: 1001,
            };
            let resolved = resolve(&sprite, fps, &temp.0.join("cache"), &|| false).unwrap();
            for frame in (0..100).rev() {
                let time = frame as u128 * 1001 * 1000;
                let audible = resolved
                    .speech
                    .iter()
                    .any(|s| s.start <= frame && frame < s.end);
                assert_eq!(
                    audible,
                    time >= 120 * numerator as u128 && time < 580 * numerator as u128
                );
            }
        }
    }
    #[test]
    fn cache_reuse_source_replacement_corruption_and_cancellation() {
        let temp = Temp::new();
        let source = temp.wav(false);
        let dir = temp.0.join("cache");
        let first = envelope(&source, &dir, &|| false).unwrap();
        let second = envelope(&source, &dir, &|| false).unwrap();
        assert!(Arc::ptr_eq(&first, &second));
        let file = fs::read_dir(&dir).unwrap().next().unwrap().unwrap().path();
        ENVELOPES
            .get()
            .unwrap()
            .lock()
            .unwrap()
            .retain(|e| e.path != source.path);
        fs::write(&file, b"broken").unwrap();
        assert_eq!(
            envelope(&source, &dir, &|| false).unwrap().levels,
            first.levels
        );
        assert!(envelope(&source, &dir, &|| true).is_err());
        fs::write(&source.path, b"not a WAV").unwrap();
        assert!(envelope(&source, &dir, &|| false).is_err());
        let mut changed = source.clone();
        changed.revision = psd::revision(&changed.path);
        assert!(envelope(&changed, &dir, &|| false).is_err());
        fs::remove_file(&source.path).unwrap();
        assert!(envelope(&source, &dir, &|| false).is_err());
    }
}
