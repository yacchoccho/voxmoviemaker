use super::{App, commands};
use crate::{Result, project::*};

impl App {
    pub(super) fn receive_import(
        &mut self,
        project: Id,
        track: Option<Id>,
        start: u64,
        results: Vec<Result<MediaAsset>>,
    ) {
        self.importing = false;
        if project != self.project.project_uuid {
            return;
        }
        let target = match track {
            Some(id) => {
                let Some(index) = self.project.tracks.iter().position(|t| t.id == id) else {
                    self.status = self.tr("stale-result");
                    return;
                };
                index
            }
            None => self.project.tracks.len(),
        };
        let mut additions = Vec::new();
        let mut error = None;
        let mut cursor = start;
        for result in results {
            let asset = match result {
                Ok(asset) => asset,
                Err(message) => {
                    error.get_or_insert(message);
                    continue;
                }
            };
            let duration = self
                .project
                .frame_rate
                .frames(if asset.kind == MediaKind::Image {
                    5.0
                } else {
                    asset.duration_seconds
                });
            let Some(end) = cursor.checked_add(duration).filter(|&end| end <= MAX_FRAME) else {
                self.status = self.tr("clip-outside-timeline");
                return;
            };
            let clip = Clip {
                id: Id::new_v4(),
                source: ClipSource::Media { asset_id: asset.id },
                timeline_start: cursor,
                duration,
                source_offset: 0,
                volume: 1.0,
                muted: false,
                effects: Default::default(),
            };
            additions.push((asset, clip));
            cursor = end;
        }
        let Some((_, first)) = additions.first() else {
            if let Some(error) = error {
                self.status = self.local_error(&error);
            }
            return;
        };
        let first_id = first.id;
        self.finish_preview_transform();
        let result = self.history.edit(&mut self.project, move |p| {
            // Reserve the entire batch so multiple files remain sequential on one layer.
            let destination = commands::placement_track(p, target, start, cursor - start)?;
            for (asset, clip) in additions {
                p.assets.push(asset);
                commands::insert(p, destination, clip)?;
            }
            Ok(destination)
        });
        let Some(destination) = self.report(result) else {
            return;
        };
        self.changed();
        self.focus_clip(first_id);
        self.status = self.tr(if destination != target {
            "inserted-safe-layer"
        } else {
            "media-inserted"
        });
        if let Some(error) = error {
            self.status = format!("{} {}", self.status, self.local_error(&error));
        }
    }
}
