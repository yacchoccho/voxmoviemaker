use super::App;
use crate::project::{commands, *};
impl App {
    pub fn insert_text(&mut self, subtitle: bool, from_voice: bool) {
        if from_voice {
            let Some(id) = self.selected else {
                return;
            };
            self.edit(|p| {
                let index = p.track_of(id).ok_or("missing-clip")?;
                if p.tracks[index].locked {
                    return Err("track-locked".into());
                }
                if let ClipSource::Voicevox { text, subtitle, .. } =
                    &mut p.clip_mut(id).ok_or("missing-clip")?.source
                    && subtitle.is_none()
                {
                    *subtitle = Some(TextSettings {
                        text: text.clone(),
                        ..Default::default()
                    });
                }
                Ok(())
            });
            return;
        }
        let text = self.tr(if subtitle {
            "subtitle-placeholder"
        } else {
            "text-placeholder"
        });
        let start = self.playhead;
        let duration = self.project.frame_rate.frames(3.0);
        let clip = Clip {
            id: Id::new_v4(),
            source: ClipSource::Text {
                settings: TextSettings {
                    text,
                    subtitle,
                    anchor: if subtitle {
                        TextAnchor::Bottom
                    } else {
                        TextAnchor::Center
                    },
                    ..Default::default()
                },
            },
            timeline_start: start,
            duration,
            source_offset: 0,
            volume: 1.0,
            muted: false,
            effects: Default::default(),
        };
        let id = clip.id;
        let preferred = self.track;
        self.finish_preview_transform();
        let result = self.history.edit(&mut self.project, |p| {
            let destination = commands::placement_track(p, preferred, start, duration)?;
            commands::insert(p, destination, clip)?;
            Ok(destination)
        });
        if let Some(destination) = self.report(result) {
            self.changed();
            self.focus_clip(id);
            if destination != preferred {
                self.status = self.tr("inserted-safe-layer");
            }
        }
    }
}
