pub use super::history::History;
use super::*;
use crate::Result;
/// Select a layer for NEW content, preserving its requested time and all existing clips.
/// Call inside History::edit so layer creation and insertion commit together.
pub fn placement_track(
    project: &mut Project,
    preferred: usize,
    start: u64,
    duration: u64,
) -> Result<usize> {
    if duration == 0
        || start
            .checked_add(duration)
            .is_none_or(|end| end > MAX_FRAME)
    {
        return Err("clip-outside-timeline".into());
    }
    if preferred > project.tracks.len() {
        return Err("invalid-track".into());
    }
    let fits = |t: &Track| !t.locked && t.clips.len() < 10_000 && t.fits(start, duration, None);
    if project.tracks.get(preferred).is_some_and(fits) {
        return Ok(preferred);
    }
    // Never silently place new foreground content behind the requested layer.
    if let Some(index) = ((preferred + 1)..project.tracks.len()).find(|&index| {
        let track = &project.tracks[index];
        fits(track) && track.enabled && track.video_visible && !track.muted
    }) {
        return Ok(index);
    }
    if project.tracks.len() >= 256 {
        return Err("no-free-layer".into());
    }
    let index = project.tracks.len();
    project.tracks.push(Track::new(index + 1));
    Ok(index)
}
pub fn insert(project: &mut Project, track: usize, clip: Clip) -> Result<()> {
    let layer = project.tracks.get_mut(track).ok_or("invalid-track")?;
    if layer.locked {
        return Err("track-locked".into());
    }
    if !layer.fits(clip.timeline_start, clip.duration, None) {
        return Err("collision".into());
    }
    layer.clips.push(clip);
    Ok(())
}
pub fn move_clip(project: &mut Project, id: Id, track: usize, start: u64) -> Result<()> {
    let old = project.track_of(id).ok_or("missing-clip")?;
    if project.tracks[old].locked {
        return Err("track-locked".into());
    }
    let mut clip = project.clip(id).ok_or("missing-clip")?.clone();
    clip.timeline_start = start;
    project.tracks[old].clips.retain(|c| c.id != id);
    insert(project, track, clip)
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClipEdge {
    Start,
    End,
}
/// Preview and commit share the same bounds: never cross a neighbor, source
/// boundary or the one-frame minimum. Still images/text/sprites have no media end.
pub fn resized_clip(project: &Project, id: Id, edge: ClipEdge, frame: u64) -> Result<Clip> {
    let index = project.track_of(id).ok_or("missing-clip")?;
    let track = &project.tracks[index];
    if track.locked {
        return Err("track-locked".into());
    }
    let mut clip = project.clip(id).ok_or("missing-clip")?.clone();
    let finite = clip
        .source
        .asset_id()
        .map(|id| project.asset(id))
        .transpose()?
        .filter(|asset| asset.kind != MediaKind::Image);
    match edge {
        ClipEdge::Start => {
            let previous = track
                .clips
                .iter()
                .filter(|c| c.id != id && c.end() <= clip.timeline_start)
                .map(Clip::end)
                .max()
                .unwrap_or(0);
            let source_start = if finite.is_some() {
                clip.timeline_start.saturating_sub(clip.source_offset)
            } else {
                0
            };
            let start = frame.clamp(previous.max(source_start), clip.end() - 1);
            clip.source_offset = if start >= clip.timeline_start {
                clip.source_offset + start - clip.timeline_start
            } else {
                clip.source_offset
                    .saturating_sub(clip.timeline_start - start)
            };
            clip.duration = clip.end() - start;
            clip.timeline_start = start;
        }
        ClipEdge::End => {
            let next = track
                .clips
                .iter()
                .filter(|c| c.id != id && c.timeline_start >= clip.end())
                .map(|c| c.timeline_start)
                .min()
                .unwrap_or(MAX_FRAME);
            let source_end = finite.map_or(MAX_FRAME, |asset| {
                project.frame_rate.frames(asset.duration_seconds)
            });
            let max_end = clip
                .timeline_start
                .saturating_add(source_end.saturating_sub(clip.source_offset))
                .min(next)
                .min(MAX_FRAME);
            if max_end <= clip.timeline_start {
                return Err("trim-outside-source".into());
            }
            clip.duration = frame.clamp(clip.timeline_start + 1, max_end) - clip.timeline_start;
        }
    }
    clip.effects.clamp_duration(clip.duration);
    Ok(clip)
}
pub fn resize_clip(project: &mut Project, id: Id, edge: ClipEdge, frame: u64) -> Result<()> {
    let clip = resized_clip(project, id, edge, frame)?;
    *project.clip_mut(id).ok_or("missing-clip")? = clip;
    Ok(())
}
pub fn split(project: &mut Project, id: Id, frame: u64) -> Result<()> {
    let track = project.track_of(id).ok_or("missing-clip")?;
    if project.tracks[track].locked {
        return Err("track-locked".into());
    }
    let clip = project.clip_mut(id).ok_or("missing-clip")?;
    if frame <= clip.timeline_start || frame >= clip.end() {
        return Err("split-outside-clip".into());
    }
    let left = frame - clip.timeline_start;
    let mut right = clip.clone();
    right.id = Id::new_v4();
    right.timeline_start = frame;
    right.duration -= left;
    right.source_offset += left;
    clip.duration = left;
    clip.effects.clamp_duration(clip.duration);
    right.effects.clamp_duration(right.duration);
    project.tracks[track].clips.push(right);
    Ok(())
}
/// Overlap the adjacent incoming clip on a new generic foreground layer.
/// Its visual dissolve reveals the outgoing picture; only outgoing audio fades.
pub fn crossfade_next(project: &mut Project, id: Id, duration: u64) -> Result<Id> {
    let index = project.track_of(id).ok_or("missing-clip")?;
    if project.tracks[index].locked {
        return Err("track-locked".into());
    }
    let outgoing = project.clip(id).ok_or("missing-clip")?.clone();
    let next = project.tracks[index]
        .clips
        .iter()
        .find(|c| c.timeline_start == outgoing.end())
        .cloned()
        .ok_or("transition-needs-adjacent")?;
    let duration = duration.min(outgoing.duration / 2).min(next.duration / 2);
    if duration == 0 {
        return Err("transition-too-short".into());
    }
    if project.tracks.len() >= 256 {
        return Err("invalid-track".into());
    }
    let mut incoming = next.clone();
    incoming.timeline_start -= duration;
    incoming.effects.transition = TransitionKind::Dissolve;
    incoming.effects.transition_frames = duration;
    incoming.effects.fade_in = 0;
    project
        .clip_mut(id)
        .ok_or("missing-clip")?
        .effects
        .audio_fade_out = duration;
    project.clip_mut(id).ok_or("missing-clip")?.effects.fade_out = 0;
    project.tracks[index].clips.retain(|c| c.id != next.id);
    let mut track = Track::new(project.tracks.len() + 1);
    track.enabled = project.tracks[index].enabled;
    track.video_visible = project.tracks[index].video_visible;
    track.muted = project.tracks[index].muted;
    track.clips.push(incoming);
    project.tracks.insert(index + 1, track);
    Ok(next.id)
}
pub fn delete(project: &mut Project, id: Id) -> Result<()> {
    let track = project.track_of(id).ok_or("missing-clip")?;
    if project.tracks[track].locked {
        return Err("track-locked".into());
    }
    project.tracks[track].clips.retain(|c| c.id != id);
    Ok(())
}
pub fn snap(
    project: &Project,
    frame: u64,
    playhead: u64,
    threshold: u64,
    except: Option<Id>,
) -> u64 {
    std::iter::once(0)
        .chain(std::iter::once(playhead))
        .chain(
            project
                .tracks
                .iter()
                .flat_map(|t| &t.clips)
                .filter(|c| Some(c.id) != except)
                .flat_map(|c| [c.timeline_start, c.end()]),
        )
        .filter(|x| x.abs_diff(frame) <= threshold)
        .min_by_key(|x| x.abs_diff(frame))
        .unwrap_or(frame)
}
