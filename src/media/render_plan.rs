use crate::{Result, assets::Storage, project::*};
use serde::Serialize;
use std::{collections::HashMap, ffi::OsString, path::PathBuf};

#[derive(Clone, Debug, Serialize)]
pub struct RenderClip {
    pub id: Id,
    pub path: PathBuf,
    pub kind: MediaKind,
    pub source_size: [u32; 2],
    pub start: u64,
    pub duration: u64,
    pub offset: u64,
    pub visual: bool,
    pub audio: bool,
    pub volume: f64,
    pub video_stream: Option<u32>,
    pub audio_stream: Option<u32>,
    pub effects: ClipEffects,
    pub text: Option<TextSettings>,
}
#[derive(Clone, Debug, Serialize)]
pub struct RenderPlan {
    pub width: u32,
    pub height: u32,
    pub fps: FrameRate,
    pub duration: u64,
    pub clips: Vec<RenderClip>,
    /// Disposable editor planes only; project/export plans always start opaque.
    pub transparent: bool,
    #[serde(skip)]
    pub scratch: PathBuf,
    #[serde(skip)]
    pub data_root: PathBuf,
}
impl RenderPlan {
    pub fn new(project: &Project, storage: &Storage) -> Result<Self> {
        let characters = crate::characters::CharacterStore::load(&storage.config);
        Self::with_characters(project, storage, &characters.library)
    }
    pub fn with_characters(
        project: &Project,
        storage: &Storage,
        characters: &crate::characters::CharacterLibrary,
    ) -> Result<Self> {
        project.validate()?;
        let mut clips = Vec::new();
        let assets: HashMap<_, _> = project.assets.iter().map(|a| (a.id, a)).collect();
        for track in &project.tracks {
            if !track.enabled {
                continue;
            }
            for clip in &track.clips {
                let asset = clip
                    .source
                    .asset_id()
                    .map(|id| {
                        assets
                            .get(&id)
                            .copied()
                            .ok_or_else(|| "missing-asset".to_string())
                    })
                    .transpose()?;
                let text = match &clip.source {
                    ClipSource::Text { settings } => Some(clip.character.as_ref().map_or_else(
                        || settings.clone(),
                        |binding| binding.text(characters, &settings.text),
                    )),
                    _ => None,
                };
                clips.push(RenderClip {
                    id: clip.id,
                    path: asset
                        .map(|a| storage.resolve(project.project_uuid, a))
                        .unwrap_or_default(),
                    kind: asset.map_or(MediaKind::Image, |a| a.kind),
                    source_size: asset.map_or([project.width, project.height], |a| {
                        if a.rotation.rem_euclid(180) == 90 {
                            [a.height, a.width]
                        } else {
                            [a.width, a.height]
                        }
                    }),
                    start: clip.timeline_start,
                    duration: clip.duration,
                    offset: clip.source_offset,
                    visual: track.video_visible && asset.is_none_or(|a| a.kind != MediaKind::Audio),
                    audio: !track.muted && !clip.muted && asset.is_some_and(|a| a.has_audio),
                    volume: clip.volume,
                    video_stream: asset.and_then(|a| a.video_stream),
                    audio_stream: asset.and_then(|a| a.audio_stream),
                    effects: clip.effects.clone(),
                    text,
                });
                // One timeline object contributes audio and an aligned transparent visual.
                // Keep it here, in this layer's order, for preview/proxy/export alike.
                let subtitle = if let ClipSource::Voicevox { text, subtitle, .. } = &clip.source {
                    if let Some(binding) = &clip.character {
                        binding
                            .subtitle_enabled
                            .then(|| binding.text(characters, text))
                    } else {
                        subtitle.clone()
                    }
                } else {
                    None
                };
                if let Some(style) = subtitle {
                    clips.push(RenderClip {
                        id: clip.id,
                        path: PathBuf::new(),
                        kind: MediaKind::Image,
                        source_size: [project.width, project.height],
                        start: clip.timeline_start,
                        duration: clip.duration,
                        offset: 0,
                        visual: track.video_visible,
                        audio: false,
                        volume: 0.0,
                        video_stream: None,
                        audio_stream: None,
                        effects: clip.effects.clone(),
                        text: Some(style),
                    });
                }
            }
        }
        Ok(Self {
            width: project.width,
            height: project.height,
            fps: project.frame_rate,
            duration: project.duration(),
            clips,
            transparent: false,
            scratch: storage.cache.join("text"),
            data_root: storage.data.clone(),
        })
    }
    pub fn visual_order(&self, frame: u64) -> Vec<Id> {
        self.clips
            .iter()
            .filter(|c| c.visual && c.start <= frame && frame < c.start + c.duration)
            .map(|c| c.id)
            .collect()
    }
    /// Preview and export use this same compiler and the same back-to-front clip order.
    /// All interpolated filter values are validated numbers; paths are separate argv entries.
    pub fn compile(&self, preview: Option<(u64, u32)>) -> Result<Compiled> {
        self.compile_mode(preview, None)
    }
    /// Full timeline with the exact export composition/mix, at bounded preview resolution.
    pub fn compile_proxy(&self, width: u32) -> Result<Compiled> {
        self.compile_mode(None, Some(width))
    }
    pub fn preview_size(&self, width: u32) -> (u32, u32) {
        let mut w = width.clamp(160, 1280) / 2 * 2;
        let mut h = (self.height as u64 * w as u64 / self.width as u64) as u32;
        if h > 720 {
            h = 720;
            w = (self.width as u64 * 720 / self.height as u64) as u32;
        }
        ((w / 2 * 2).max(2), (h / 2 * 2).max(2))
    }
    fn compile_mode(&self, preview: Option<(u64, u32)>, proxy: Option<u32>) -> Result<Compiled> {
        let (width, height) = if let Some(w) = preview.map(|(_, w)| w).or(proxy) {
            self.preview_size(w)
        } else {
            (self.width, self.height)
        };
        let total = self
            .fps
            .timestamp(if preview.is_some() { 1 } else { self.duration });
        let mut args: Vec<OsString> = [
            "-hide_banner",
            "-nostdin",
            "-v",
            "error",
            "-filter_complex_threads",
            "1",
            "-threads",
            "1",
        ]
        .into_iter()
        .map(Into::into)
        .collect();
        let transparent = preview.is_some() && self.transparent;
        let background = if transparent { "black@0" } else { "black" };
        let overlay_format = if transparent { ":format=auto" } else { "" };
        let mut filters = vec![format!(
            "color=c={background}:s={width}x{height}:r={}:d={total},format=rgba[base]",
            self.fps.ffmpeg()
        )];
        let mut video = "base".to_string();
        let mut audios = Vec::new();
        let mut input = 0;
        let mut text_files = Vec::new();
        for clip in &self.clips {
            let mut start = clip.start;
            let mut offset = clip.offset;
            let mut duration = clip.duration;
            let audio = clip.audio && preview.is_none();
            if let Some((frame, _)) = preview {
                if !clip.visual || frame < clip.start || frame >= clip.start + clip.duration {
                    continue;
                }
                offset += frame - clip.start;
                start = 0;
                duration = 1;
            }
            if !clip.visual && !audio {
                continue;
            }
            let path = if let Some(text) = &clip.text {
                let file = super::text::temporary_image(
                    text,
                    width,
                    height,
                    self.width,
                    &self.scratch,
                    &self.data_root,
                )?;
                let path = file.path.clone();
                text_files.push(file);
                path
            } else {
                clip.path.clone()
            };
            if !path.is_file() {
                return Err(format!("missing-asset: {}", path.display()));
            }
            args.extend([
                OsString::from("-protocol_whitelist"),
                "file,pipe".into(),
                "-threads".into(),
                "1".into(),
            ]);
            if clip.kind == MediaKind::Image {
                args.extend([
                    OsString::from("-loop"),
                    "1".into(),
                    "-framerate".into(),
                    self.fps.ffmpeg().into(),
                ]);
            } else {
                args.extend([OsString::from("-ss"), self.fps.timestamp(offset).into()]);
            }
            args.extend([
                OsString::from("-t"),
                self.fps.timestamp(duration).into(),
                "-i".into(),
                path.into_os_string(),
            ]);
            let start_time = self.fps.timestamp(start);
            let duration_time = self.fps.timestamp(duration);
            let video_stream = clip
                .video_stream
                .map(|n| n.to_string())
                .unwrap_or_else(|| "v:0".into());
            let audio_stream = clip
                .audio_stream
                .map(|n| n.to_string())
                .unwrap_or_else(|| "a:0".into());
            if clip.visual {
                let relative = preview.map_or(0, |(frame, _)| frame - clip.start);
                let alpha = if preview.is_some() {
                    clip.effects.visual_alpha(relative, clip.duration)
                } else {
                    clip.effects.opacity
                };
                let sw = ((width as f64 * clip.effects.scale).round() as u32).max(2);
                let sh = ((height as f64 * clip.effects.scale).round() as u32).max(2);
                let mut effect = format!("colorchannelmixer=aa={alpha}");
                if preview.is_none() {
                    for (kind, frames, frame) in [
                        ("in", clip.effects.visual_fade_in(), 0),
                        (
                            "out",
                            clip.effects.fade_out,
                            clip.duration.saturating_sub(clip.effects.fade_out),
                        ),
                    ] {
                        if frames > 0 {
                            effect.push_str(&format!(
                                ",fade=t={kind}:st={}:d={}:alpha=1",
                                self.fps.timestamp(frame),
                                self.fps.timestamp(frames)
                            ));
                        }
                    }
                }
                filters.push(format!("[{input}:{video_stream}]trim=duration={duration_time},setpts=PTS-STARTPTS,scale={width}:{height}:force_original_aspect_ratio=decrease,format=rgba,pad={width}:{height}:(ow-iw)/2:(oh-ih)/2:color=black@0,scale={sw}:{sh},setsar=1,{effect},setpts=PTS+{start_time}/TB[v{input}]"));
                let x = clip.effects.x * width as f64 / self.width as f64;
                let y = clip.effects.y * height as f64 / self.height as f64;
                let slide = if clip.effects.transition == TransitionKind::SlideLeft
                    && clip.effects.transition_frames > 0
                {
                    if preview.is_some() {
                        format!("+{}", clip.effects.slide_offset(relative, width as f64))
                    } else {
                        format!(
                            "+W*max(0,1-(t-{start_time})/{})",
                            self.fps.timestamp(clip.effects.transition_frames)
                        )
                    }
                } else {
                    String::new()
                };
                filters.push(format!("[{video}][v{input}]overlay=x='(W-w)/2+{x}{slide}':y='(H-h)/2+{y}'{overlay_format}:eof_action=pass:repeatlast=0:shortest=0:enable='gte(t,{start_time})*lt(t,{})'[o{input}]", self.fps.timestamp(start + duration)));
                video = format!("o{input}");
            }
            if audio {
                // Delay in samples avoids millisecond rounding at fractional frame rates.
                let delay = self.fps.samples(start, 48_000);
                let incoming = clip.effects.audio_fade_in.max(
                    if clip.effects.transition != TransitionKind::None {
                        clip.effects.transition_frames
                    } else {
                        0
                    },
                );
                let mut fades = String::new();
                for (kind, frames, frame) in [
                    ("in", incoming, 0),
                    (
                        "out",
                        clip.effects.audio_fade_out,
                        clip.duration.saturating_sub(clip.effects.audio_fade_out),
                    ),
                ] {
                    if frames > 0 {
                        fades.push_str(&format!(
                            ",afade=t={kind}:st={}:d={}",
                            self.fps.timestamp(frame),
                            self.fps.timestamp(frames)
                        ));
                    }
                }
                filters.push(format!("[{input}:{audio_stream}]atrim=duration={duration_time},asetpts=PTS-STARTPTS,aresample=48000,aformat=sample_fmts=fltp:channel_layouts=stereo,volume={}{fades},adelay={delay}S:all=1[a{input}]", clip.volume));
                audios.push(format!("[a{input}]"));
            }
            input += 1;
        }
        let pixel_format = if transparent { "rgba" } else { "yuv420p" };
        filters.push(format!("[{video}]format={pixel_format}[vout]"));
        if preview.is_none() {
            if audios.is_empty() {
                filters.push(format!(
                    "anullsrc=r=48000:cl=stereo,atrim=duration={total}[aout]"
                ));
            } else {
                filters.push(format!("{}amix=inputs={}:duration=longest:normalize=0,apad,atrim=duration={total}[aout]", audios.join(""), audios.len()));
            }
        }
        let graph = filters.join(";");
        let graph_file = if graph.len() >= 64 * 1024 {
            let file = super::filter_graph::FilterGraphFile::create(
                &graph,
                &self.scratch,
                &self.data_root,
            )?;
            args.extend([
                OsString::from("-filter_complex_script"),
                file.path.as_os_str().to_owned(),
            ]);
            Some(file)
        } else {
            args.extend([OsString::from("-filter_complex"), graph.into()]);
            None
        };
        args.extend([OsString::from("-map"), "[vout]".into()]);
        if preview.is_none() {
            args.extend([OsString::from("-map"), "[aout]".into()]);
        }
        args.extend([OsString::from("-threads"), "1".into()]);
        Ok(Compiled {
            args,
            width,
            height,
            _text_files: text_files,
            _graph_file: graph_file,
        })
    }
}
pub struct Compiled {
    pub args: Vec<OsString>,
    pub width: u32,
    pub height: u32,
    _text_files: Vec<super::text::TextImage>,
    _graph_file: Option<super::filter_graph::FilterGraphFile>,
}
