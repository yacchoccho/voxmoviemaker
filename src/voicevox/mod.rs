use crate::{Result, assets::Storage, project::*};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{io::Cursor, net::IpAddr, time::Duration};

#[derive(Clone, Debug, Deserialize)]
pub struct Style {
    pub id: u32,
    pub name: String,
    #[serde(default, rename = "type")]
    pub style_type: Option<String>,
}
impl Style {
    pub fn is_talk(&self) -> bool {
        matches!(
            self.style_type.as_deref(),
            None | Some("talk" | "streaming_talk")
        )
    }
}
#[derive(Clone, Debug, Deserialize)]
pub struct Speaker {
    pub name: String,
    pub speaker_uuid: String,
    pub styles: Vec<Style>,
}
#[derive(Clone, Debug)]
pub struct Connection {
    pub version: String,
    pub speakers: Vec<Speaker>,
}
pub struct Client {
    base: String,
    agent: ureq::Agent,
}
pub fn validate_url(base: &str, allow_remote: bool) -> Result<()> {
    let uri: ureq::http::Uri = base.parse().map_err(|_| "invalid-url")?;
    // No TLS stack is bundled in this first version. Remote HTTP requires an explicit setting.
    if uri.scheme_str() != Some("http")
        || uri.authority().is_none()
        || base.contains('@')
        || base.contains('#')
        || uri
            .path_and_query()
            .is_some_and(|p| p.as_str() != "/" && !p.as_str().is_empty())
    {
        return Err("invalid-url".into());
    }
    let host = uri
        .host()
        .ok_or("invalid-url")?
        .trim_start_matches('[')
        .trim_end_matches(']');
    let local = host.eq_ignore_ascii_case("localhost")
        || host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback());
    if !local && !allow_remote {
        return Err("remote-voicevox-disabled".into());
    }
    Ok(())
}
impl Client {
    pub fn new(base: &str, allow_remote: bool) -> Result<Self> {
        validate_url(base, allow_remote)?;
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(60)))
            .max_redirects(0)
            .proxy(None)
            .build()
            .into();
        Ok(Self {
            base: base.trim_end_matches('/').into(),
            agent,
        })
    }
    pub fn connect(&self) -> Result<Connection> {
        let version: String = self
            .agent
            .get(format!("{}/version", self.base))
            .call()
            .map_err(|e| e.to_string())?
            .body_mut()
            .with_config()
            .limit(4096)
            .read_json()
            .map_err(|e| e.to_string())?;
        let mut speakers: Vec<Speaker> = self
            .agent
            .get(format!("{}/speakers", self.base))
            .call()
            .map_err(|e| e.to_string())?
            .body_mut()
            .with_config()
            .limit(4 * 1024 * 1024)
            .read_json()
            .map_err(|e| e.to_string())?;
        for speaker in &mut speakers {
            speaker.styles.retain(Style::is_talk);
        }
        speakers.retain(|s| !s.styles.is_empty());
        Ok(Connection { version, speakers })
    }
    pub fn synthesize(
        &self,
        text: &str,
        settings: &VoicevoxSettings,
    ) -> Result<(Vec<u8>, f64, u32)> {
        settings.validate()?;
        if text.trim().is_empty() || text.len() > 100_000 {
            return Err("invalid-text".into());
        }
        let mut query: Value = self
            .agent
            .post(format!("{}/audio_query", self.base))
            .query("text", text)
            .query("speaker", settings.speaker.to_string())
            .send_empty()
            .map_err(|e| e.to_string())?
            .body_mut()
            .with_config()
            .limit(8 * 1024 * 1024)
            .read_json()
            .map_err(|e| e.to_string())?;
        if !query.is_object() || !query["accent_phrases"].is_array() {
            return Err("invalid-audio-query".into());
        }
        for (key, value) in [
            ("speedScale", settings.speed),
            ("pitchScale", settings.pitch),
            ("intonationScale", settings.intonation),
            ("volumeScale", settings.volume),
            ("prePhonemeLength", settings.pre_phoneme),
            ("postPhonemeLength", settings.post_phoneme),
        ] {
            query[key] = json!(value);
        }
        query["outputSamplingRate"] = json!(48000);
        query["outputStereo"] = json!(false);
        let wav = self
            .agent
            .post(format!("{}/synthesis", self.base))
            .query("speaker", settings.speaker.to_string())
            .send_json(&query)
            .map_err(|e| e.to_string())?
            .body_mut()
            .with_config()
            .limit(128 * 1024 * 1024)
            .read_to_vec()
            .map_err(|e| e.to_string())?;
        let (duration, sample_rate) = wav_info(&wav)?;
        Ok((wav, duration, sample_rate))
    }
}
pub fn wav_info(bytes: &[u8]) -> Result<(f64, u32)> {
    let mut reader = hound::WavReader::new(Cursor::new(bytes)).map_err(|e| e.to_string())?;
    let spec = reader.spec();
    if spec.sample_rate == 0 || spec.channels == 0 || reader.duration() == 0 {
        return Err("invalid-wav".into());
    }
    // Validate the final frame as well as the header, without loading all samples.
    reader
        .seek(reader.duration() - 1)
        .map_err(|e| e.to_string())?;
    match spec.sample_format {
        hound::SampleFormat::Float => {
            for sample in reader.samples::<f32>().take(spec.channels as usize) {
                sample.map_err(|e| e.to_string())?;
            }
        }
        hound::SampleFormat::Int => {
            for sample in reader.samples::<i32>().take(spec.channels as usize) {
                sample.map_err(|e| e.to_string())?;
            }
        }
    }
    Ok((
        reader.duration() as f64 / spec.sample_rate as f64,
        spec.sample_rate,
    ))
}
pub fn persist(
    storage: &Storage,
    project: Id,
    wav: &[u8],
    settings: &VoicevoxSettings,
) -> Result<MediaAsset> {
    let (duration_seconds, sample_rate) = wav_info(wav)?;
    let id = Id::new_v4();
    storage.save_voice(project, id, wav)?;
    Ok(MediaAsset {
        id,
        name: format!(
            "VOICEVOX: {} / {}",
            settings.speaker_name, settings.style_name
        ),
        location: AssetLocation::Voice(id),
        kind: MediaKind::Audio,
        duration_seconds,
        width: 0,
        height: 0,
        has_audio: true,
        sample_rate: Some(sample_rate),
        frame_rate: None,
        codec: "pcm".into(),
        start_time: 0.0,
        rotation: 0,
        video_stream: None,
        audio_stream: Some(0),
    })
}
