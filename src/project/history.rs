//! Transactional edits with reversible collection deltas and a bounded history budget.
use super::*;
use crate::Result;
use serde::Serialize;
use std::{
    collections::{HashMap, VecDeque},
    io::{self, Write},
};

#[derive(Serialize)]
struct Splice<T> {
    index: usize,
    removed: Vec<T>,
    inserted: Vec<T>,
}
impl<T: Clone + PartialEq> Splice<T> {
    fn between(before: &[T], after: &[T]) -> Self {
        let index = before.iter().zip(after).take_while(|(a, b)| a == b).count();
        let suffix = before[index..]
            .iter()
            .rev()
            .zip(after[index..].iter().rev())
            .take_while(|(a, b)| a == b)
            .count();
        Self {
            index,
            removed: before[index..before.len() - suffix].to_vec(),
            inserted: after[index..after.len() - suffix].to_vec(),
        }
    }
    fn apply(&self, values: &mut Vec<T>, forward: bool) -> Result<()> {
        let (removed, inserted) = if forward {
            (&self.removed, &self.inserted)
        } else {
            (&self.inserted, &self.removed)
        };
        let end = self
            .index
            .checked_add(removed.len())
            .ok_or("history-outdated")?;
        if values.get(self.index..end) != Some(removed.as_slice()) {
            return Err("history-outdated".into());
        }
        values.splice(self.index..end, inserted.iter().cloned());
        Ok(())
    }
}
#[derive(Clone, Serialize)]
struct ProjectHeader {
    schema_version: u32,
    project_uuid: Id,
    width: u32,
    height: u32,
    frame_rate: FrameRate,
}
impl ProjectHeader {
    fn new(p: &Project) -> Self {
        Self {
            schema_version: p.schema_version,
            project_uuid: p.project_uuid,
            width: p.width,
            height: p.height,
            frame_rate: p.frame_rate,
        }
    }
    fn apply(&self, p: &mut Project) {
        p.schema_version = self.schema_version;
        p.project_uuid = self.project_uuid;
        p.width = self.width;
        p.height = self.height;
        p.frame_rate = self.frame_rate;
    }
}
#[derive(Clone, Serialize)]
struct TrackHeader {
    name: String,
    enabled: bool,
    video_visible: bool,
    muted: bool,
    locked: bool,
}
impl TrackHeader {
    fn new(t: &Track) -> Self {
        Self {
            name: t.name.clone(),
            enabled: t.enabled,
            video_visible: t.video_visible,
            muted: t.muted,
            locked: t.locked,
        }
    }
    fn apply(&self, t: &mut Track) {
        t.name = self.name.clone();
        t.enabled = self.enabled;
        t.video_visible = self.video_visible;
        t.muted = self.muted;
        t.locked = self.locked;
    }
}
#[derive(Serialize)]
struct TrackDelta {
    id: Id,
    before: Option<TrackHeader>,
    after: Option<TrackHeader>,
    clips: Splice<Clip>,
}
#[derive(Serialize)]
struct Delta {
    before: ProjectHeader,
    after: ProjectHeader,
    assets: Splice<MediaAsset>,
    order_before: Vec<Id>,
    order_after: Vec<Id>,
    tracks: Vec<TrackDelta>,
}
impl Delta {
    fn between(before: &Project, after: &Project) -> Self {
        let old: HashMap<_, _> = before.tracks.iter().map(|t| (t.id, t)).collect();
        let new: HashMap<_, _> = after.tracks.iter().map(|t| (t.id, t)).collect();
        let mut tracks = Vec::new();
        for t in &before.tracks {
            let next = new.get(&t.id).copied();
            if next != Some(t) {
                tracks.push(TrackDelta {
                    id: t.id,
                    before: Some(TrackHeader::new(t)),
                    after: next.map(TrackHeader::new),
                    clips: Splice::between(&t.clips, next.map_or(&[], |t| t.clips.as_slice())),
                });
            }
        }
        for t in &after.tracks {
            if !old.contains_key(&t.id) {
                tracks.push(TrackDelta {
                    id: t.id,
                    before: None,
                    after: Some(TrackHeader::new(t)),
                    clips: Splice::between(&[], &t.clips),
                });
            }
        }
        Self {
            before: ProjectHeader::new(before),
            after: ProjectHeader::new(after),
            assets: Splice::between(&before.assets, &after.assets),
            order_before: before.tracks.iter().map(|t| t.id).collect(),
            order_after: after.tracks.iter().map(|t| t.id).collect(),
            tracks,
        }
    }
    fn apply(&self, project: &mut Project, forward: bool) -> Result<()> {
        if forward { &self.after } else { &self.before }.apply(project);
        self.assets.apply(&mut project.assets, forward)?;
        for change in &self.tracks {
            let header = if forward {
                &change.after
            } else {
                &change.before
            };
            let Some(header) = header else {
                project.tracks.retain(|t| t.id != change.id);
                continue;
            };
            if !project.tracks.iter().any(|t| t.id == change.id) {
                project.tracks.push(Track {
                    id: change.id,
                    name: String::new(),
                    enabled: true,
                    video_visible: true,
                    muted: false,
                    locked: false,
                    clips: Vec::new(),
                });
            }
            let t = project
                .tracks
                .iter_mut()
                .find(|t| t.id == change.id)
                .ok_or("history-outdated")?;
            header.apply(t);
            change.clips.apply(&mut t.clips, forward)?;
        }
        let order = if forward {
            &self.order_after
        } else {
            &self.order_before
        };
        let positions: HashMap<_, _> = order.iter().enumerate().map(|(i, id)| (*id, i)).collect();
        if project.tracks.len() != order.len()
            || project
                .tracks
                .iter()
                .any(|t| !positions.contains_key(&t.id))
        {
            return Err("history-outdated".into());
        }
        project.tracks.sort_unstable_by_key(|t| positions[&t.id]);
        project.validate()
    }
    fn size(&self) -> Result<usize> {
        struct Count(usize);
        impl Write for Count {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                self.0 = self.0.saturating_add(bytes.len());
                Ok(bytes.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let mut size = Count(0);
        serde_json::to_writer(&mut size, self).map_err(|e| e.to_string())?;
        Ok(size.0.saturating_add(std::mem::size_of::<Self>()))
    }
}
struct Entry {
    change: Delta,
    bytes: usize,
}
pub struct History {
    undo: VecDeque<Entry>,
    redo: Vec<Entry>,
    group: Option<Project>,
    bytes: usize,
    max_entries: usize,
    max_bytes: usize,
}
impl Default for History {
    fn default() -> Self {
        Self::with_limits(50, 64 * 1024 * 1024)
    }
}
impl History {
    /// Budget is measured as encoded delta bytes plus fixed delta metadata.
    pub fn with_limits(max_entries: usize, max_bytes: usize) -> Self {
        Self {
            undo: VecDeque::new(),
            redo: Vec::new(),
            group: None,
            bytes: 0,
            max_entries,
            max_bytes,
        }
    }
    pub fn retained_bytes(&self) -> usize {
        self.bytes
    }
    pub fn undo_len(&self) -> usize {
        self.undo.len()
    }
    pub fn is_grouping(&self) -> bool {
        self.group.is_some()
    }
    pub fn begin_group(&mut self, project: &Project) {
        if self.group.is_none() {
            self.group = Some(project.clone());
        }
    }
    pub fn finish_group(&mut self, project: &Project) -> Result<()> {
        if let Some(before) = self.group.take()
            && before != *project
        {
            self.record(Delta::between(&before, project))?;
        }
        Ok(())
    }
    fn record(&mut self, change: Delta) -> Result<()> {
        let bytes = change.size()?;
        self.bytes = self
            .bytes
            .saturating_sub(self.redo.iter().map(|e| e.bytes).sum::<usize>());
        self.redo.clear();
        if bytes > self.max_bytes || self.max_entries == 0 {
            self.undo.clear();
            self.bytes = 0;
            return Ok(());
        }
        self.bytes = self.bytes.saturating_add(bytes);
        self.undo.push_back(Entry { change, bytes });
        while self.bytes > self.max_bytes || self.undo.len() > self.max_entries {
            if let Some(old) = self.undo.pop_front() {
                self.bytes = self.bytes.saturating_sub(old.bytes);
            }
        }
        Ok(())
    }
    pub fn edit<T>(
        &mut self,
        project: &mut Project,
        edit: impl FnOnce(&mut Project) -> Result<T>,
    ) -> Result<T> {
        let mut next = project.clone();
        let result = edit(&mut next)?;
        if next != *project {
            next.validate()?;
            if self.group.is_none() {
                self.record(Delta::between(project, &next))?;
            }
            *project = next;
        }
        Ok(result)
    }
    pub fn undo(&mut self, project: &mut Project) {
        if self.finish_group(project).is_err() {
            return;
        }
        if let Some(entry) = self.undo.pop_back() {
            let mut next = project.clone();
            if entry.change.apply(&mut next, false).is_ok() {
                *project = next;
                self.redo.push(entry);
            } else {
                self.undo.push_back(entry);
            }
        }
    }
    pub fn redo(&mut self, project: &mut Project) {
        if self.finish_group(project).is_err() {
            return;
        }
        if let Some(entry) = self.redo.pop() {
            let mut next = project.clone();
            if entry.change.apply(&mut next, true).is_ok() {
                *project = next;
                self.undo.push_back(entry);
            } else {
                self.redo.push(entry);
            }
        }
    }
}
