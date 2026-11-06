//! Bounded read-only PSD decoding. No PSD writer is called by the application.
use super::{Sprite, SpriteRender};
use crate::{Result, project::FrameRate};
use apeireth_image_psd::{BlendMode, ColorMode, LayerNode, PsdFile, SectionType};
use image::{Pixel, RgbaImage};
use std::{
    collections::{BTreeSet, HashMap, VecDeque},
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
};

const FILE_LIMIT: u64 = 64 * 1024 * 1024;
const PIXEL_LIMIT: u64 = 192 * 1024 * 1024;
const IMAGE_CACHE_LIMIT: usize = 64 * 1024 * 1024;

#[derive(Debug)]
pub struct Node {
    pub key: String,
    pub index: usize,
    pub name: String,
    pub visible: bool,
    pub opacity: u8,
    pub pass_through: bool,
    pub group: bool,
    pub rect: [i32; 4],
    pub children: Vec<Node>,
}
pub struct Document {
    pub width: u32,
    pub height: u32,
    pub nodes: Vec<Node>,
    file: PsdFile,
    decoded: Mutex<PixelCache>,
}
#[derive(Default)]
struct PixelCache {
    entries: VecDeque<(usize, Arc<RgbaImage>)>,
    bytes: usize,
}
#[derive(Clone, PartialEq, Eq)]
struct Stamp {
    path: PathBuf,
    len: u64,
    revision: [u64; 7],
}
type DocumentCache = Mutex<VecDeque<(Stamp, Arc<Document>)>>;
static DOCUMENTS: OnceLock<DocumentCache> = OnceLock::new();

/// Includes inode/ctime so atomic saves that preserve length/mtime still invalidate caches.
pub fn revision(path: &Path) -> Option<[u64; 7]> {
    use std::os::unix::fs::MetadataExt;
    let m = fs::metadata(path).ok()?;
    Some([
        m.dev(),
        m.ino(),
        m.len(),
        m.mtime() as u64,
        m.mtime_nsec() as u64,
        m.ctime() as u64,
        m.ctime_nsec() as u64,
    ])
}
pub fn load(path: &Path) -> Result<Arc<Document>> {
    crate::assets::require_utf8_path(path)?;
    let metadata =
        fs::metadata(path).map_err(|e| format!("sprite-psd-missing: {}: {e}", path.display()))?;
    if !metadata.is_file() || metadata.len() > FILE_LIMIT {
        return Err("sprite-too-large".into());
    }
    let stamp = Stamp {
        path: path.canonicalize().map_err(|e| e.to_string())?,
        len: metadata.len(),
        revision: revision(path).ok_or("sprite-psd-missing")?,
    };
    // Serialize parses to avoid duplicate large allocations from preview/export/settings workers.
    let mut cache = DOCUMENTS
        .get_or_init(Default::default)
        .lock()
        .map_err(|_| "sprite-cache-error")?;
    if let Some(index) = cache.iter().position(|(key, _)| key == &stamp) {
        let entry = cache.remove(index).ok_or("sprite-cache-error")?;
        let doc = entry.1.clone();
        cache.push_back(entry);
        return Ok(doc);
    }
    let mut bytes = Vec::new();
    fs::File::open(path)
        .map_err(|e| e.to_string())?
        .take(FILE_LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > FILE_LIMIT {
        return Err("sprite-too-large".into());
    }
    let doc = Arc::new(Document::from_bytes(&bytes)?);
    let after = fs::metadata(path).map_err(|e| e.to_string())?;
    if after.len() != stamp.len || revision(path) != Some(stamp.revision) {
        return Err("sprite-psd-changed".into());
    }
    cache.retain(|(key, _)| key.path != stamp.path);
    while cache.len() >= 2 {
        cache.pop_front();
    }
    cache.push_back((stamp, doc.clone()));
    Ok(doc)
}
impl Document {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        if bytes.len() as u64 > FILE_LIMIT {
            return Err("sprite-too-large".into());
        }
        let file = PsdFile::from_bytes(bytes).map_err(|e| format!("sprite-psd-invalid: {e}"))?;
        let h = &file.header;
        if h.color_mode != ColorMode::Rgb || h.depth != 8 {
            return Err("sprite-psd-unsupported: RGB 8-bit required".into());
        }
        let canvas = h.width as u64 * h.height as u64 * 4;
        if canvas == 0 || canvas > 64 * 1024 * 1024 || file.layers().len() > 2048 {
            return Err("sprite-too-large".into());
        }
        let mut pixels = 0u64;
        let mut depth = 0usize;
        let mut max_depth = 0;
        let mut ids = HashMap::new();
        for record in file.layers() {
            match record.section_type() {
                SectionType::BoundingDivider => {
                    depth += 1;
                    max_depth = max_depth.max(depth);
                }
                SectionType::OpenFolder | SectionType::ClosedFolder => {
                    depth = depth.saturating_sub(1);
                }
                _ => {}
            }
            if let Some(id) = record.layer_id() {
                *ids.entry(id).or_insert(0usize) += 1;
            }
            let (w, h) = record
                .rect
                .size()
                .map_err(|e| format!("sprite-psd-invalid: {e}"))?;
            pixels = pixels.saturating_add((w as u64).saturating_mul(h as u64).saturating_mul(4));
            if pixels > PIXEL_LIMIT || max_depth > 32 {
                return Err("sprite-too-large".into());
            }
            if record.section_type() == SectionType::BoundingDivider {
                continue;
            }
            let unsupported = if record.clipping != 0 {
                Some("clipping mask")
            } else if record.layer_mask().is_some() || record.channels.iter().any(|c| c.id < -1) {
                Some("layer mask")
            } else if !matches!(
                record.blend_mode,
                BlendMode::Normal | BlendMode::PassThrough
            ) {
                Some("blend mode")
            } else if record.blocks.iter().any(|b| {
                [
                    *b"vmsk", *b"vsms", *b"lrFX", *b"lfx2", *b"lfxs", *b"TySh", *b"SoLd", *b"SoLE",
                    *b"SoCo", *b"GdFl", *b"PtFl", *b"levl", *b"curv", *b"brit", *b"hue2", *b"blnc",
                    *b"selc", *b"nvrt", *b"thrs", *b"post", *b"mixr", *b"grdm", *b"expA", *b"vibA",
                    *b"blwh", *b"phfl",
                ]
                .contains(&b.key)
            }) {
                Some("effects, vector, smart object or adjustment layer")
            } else if record
                .blending_ranges
                .ranges()
                .iter()
                .any(|r| r.source != [0, 0, 255, 255] || r.dest != [0, 0, 255, 255])
            {
                Some("blend-if")
            } else {
                None
            };
            if let Some(feature) = unsupported {
                return Err(format!(
                    "sprite-psd-unsupported: {} ({feature})",
                    record.name()
                ));
            }
        }
        if depth != 0 {
            return Err("sprite-psd-invalid: unbalanced groups".into());
        }
        fn nodes(
            file: &PsdFile,
            tree: &[LayerNode],
            ids: &HashMap<u32, usize>,
            canvas: u64,
            isolated: u64,
        ) -> Result<Vec<Node>> {
            tree.iter()
                .map(|node| {
                    let r = &file.layers()[node.index()];
                    let group = node.is_group();
                    if !group && r.blend_mode != BlendMode::Normal {
                        return Err(format!(
                            "sprite-psd-unsupported: {} (raster blend mode)",
                            r.name()
                        ));
                    }

                    if !group && r.flags.pixel_data_irrelevant() {
                        return Err(format!(
                            "sprite-psd-unsupported: {} (non-raster layer)",
                            r.name()
                        ));
                    }
                    if group && r.fill_opacity() != 255 {
                        return Err(format!(
                            "sprite-psd-unsupported: {} (group fill opacity)",
                            r.name()
                        ));
                    }
                    let blend = r
                        .section_divider()
                        .and_then(|s| s.blend_mode)
                        .unwrap_or(r.blend_mode);
                    if !matches!(blend, BlendMode::Normal | BlendMode::PassThrough) {
                        return Err(format!(
                            "sprite-psd-unsupported: {} (group blend)",
                            r.name()
                        ));
                    }
                    let pass = blend == BlendMode::PassThrough && r.opacity == 255;
                    let isolated = isolated + u64::from(group && !pass);
                    if canvas.saturating_mul(isolated + 1) > 128 * 1024 * 1024 {
                        return Err("sprite-too-large".into());
                    }
                    let key = match r.layer_id().filter(|id| ids.get(id) == Some(&1)) {
                        Some(id) => format!("id:{id}"),
                        None => format!("index:{}:{}", node.index(), r.name()),
                    };
                    Ok(Node {
                        key,
                        index: node.index(),
                        name: r.name(),
                        visible: r.is_visible(),
                        opacity: r.opacity,
                        pass_through: pass,
                        group,
                        rect: [r.rect.left, r.rect.top, r.rect.right, r.rect.bottom],
                        children: nodes(file, node.children(), ids, canvas, isolated)?,
                    })
                })
                .collect()
        }
        let nodes = nodes(&file, &file.layer_tree_max_depth(32), &ids, canvas, 0)?;
        Ok(Self {
            width: file.header.width,
            height: file.header.height,
            nodes,
            file,
            decoded: Mutex::new(PixelCache::default()),
        })
    }
    pub fn all_nodes(&self) -> Vec<&Node> {
        fn visit<'a>(nodes: &'a [Node], out: &mut Vec<&'a Node>) {
            for node in nodes {
                out.push(node);
                visit(&node.children, out);
            }
        }
        let mut out = Vec::new();
        visit(&self.nodes, &mut out);
        out
    }
    fn expanded(
        &self,
        keys: impl IntoIterator<Item = impl AsRef<str>>,
    ) -> Result<BTreeSet<String>> {
        let all = self.all_nodes();
        fn add(n: &Node, out: &mut BTreeSet<String>) {
            out.insert(n.key.clone());
            for c in &n.children {
                add(c, out);
            }
        }
        let mut out = BTreeSet::new();
        for key in keys {
            let n = all
                .iter()
                .find(|n| n.key == key.as_ref())
                .ok_or_else(|| format!("sprite-layer-missing: {}", key.as_ref()))?;
            add(n, &mut out);
        }
        Ok(out)
    }
    pub fn validate_settings(&self, sprite: &Sprite) -> Result<()> {
        sprite.validate()?;
        self.expanded(sprite.visibility.keys())?;
        let mouth = self.animation_scope(sprite.mouth.keys())?;
        let eyes = self.animation_scope(sprite.blink.keys())?;
        if !mouth.is_disjoint(&eyes) {
            return Err("sprite-animation-conflict".into());
        }
        Ok(())
    }
    fn node_path(&self, key: &str) -> Result<Vec<&Node>> {
        fn find<'a>(nodes: &'a [Node], key: &str, path: &mut Vec<&'a Node>) -> bool {
            for node in nodes {
                path.push(node);
                if node.key == key || find(&node.children, key, path) {
                    return true;
                }
                path.pop();
            }
            false
        }
        let mut path = Vec::new();
        if !find(&self.nodes, key, &mut path) {
            return Err(format!("sprite-layer-missing: {key}"));
        }
        Ok(path)
    }
    /// PSDTool '*' siblings are alternative parts. Include their entire branches
    /// in the controlled set so the original open mouth/eyes cannot remain behind
    /// a selected replacement. Unrelated body/hair groups are left untouched.
    fn animation_scope<'a>(
        &self,
        keys: impl IntoIterator<Item = &'a String>,
    ) -> Result<BTreeSet<String>> {
        let mut scope = BTreeSet::new();
        for key in keys {
            scope.extend(self.expanded([key])?);
            let path = self.node_path(key)?;
            let mut siblings = self.nodes.as_slice();
            for node in path {
                if node.name.starts_with('*') {
                    scope.extend(
                        self.expanded(
                            siblings
                                .iter()
                                .filter(|s| s.name.starts_with('*'))
                                .map(|s| &s.key),
                        )?,
                    );
                }
                siblings = &node.children;
            }
        }
        Ok(scope)
    }
    fn selected_animation_layers(
        &self,
        sprite: &Sprite,
        keys: &BTreeSet<String>,
    ) -> Result<BTreeSet<String>> {
        fn defaults(node: &Node, sprite: &Sprite, visible: &mut BTreeSet<String>) {
            for child in &node.children {
                if sprite
                    .visibility
                    .get(&child.key)
                    .copied()
                    .unwrap_or(child.visible)
                {
                    visible.insert(child.key.clone());
                    defaults(child, sprite, visible);
                }
            }
        }
        let mut selected = BTreeSet::new();
        let mut explicit = BTreeSet::new();
        for key in keys {
            let path = self.node_path(key)?;
            for node in path {
                explicit.insert(node.key.clone());
                selected.insert(node.key.clone());
                // A group means its initial visible composition, not every hidden
                // variant. Restoring an alternative ancestor also restores companion
                // parts (e.g. white eyes when selecting a pupil in an open-eye set).
                if node.group && (node.key == *key || node.name.starts_with('*')) {
                    defaults(node, sprite, &mut selected);
                }
            }
        }
        fn alternatives(
            doc: &Document,
            nodes: &[Node],
            explicit: &BTreeSet<String>,
            selected: &mut BTreeSet<String>,
        ) -> Result<()> {
            if nodes
                .iter()
                .any(|n| n.name.starts_with('*') && explicit.contains(&n.key))
            {
                for node in nodes
                    .iter()
                    .filter(|n| n.name.starts_with('*') && !explicit.contains(&n.key))
                {
                    let branch = doc.expanded([&node.key])?;
                    selected.retain(|key| !branch.contains(key));
                }
            }
            for node in nodes {
                alternatives(doc, &node.children, explicit, selected)?;
            }
            Ok(())
        }
        alternatives(self, &self.nodes, &explicit, &mut selected)?;
        Ok(selected)
    }
    pub fn layer_pixels(&self, index: usize) -> Result<Arc<RgbaImage>> {
        let mut cache = self.decoded.lock().map_err(|_| "sprite-cache-error")?;
        if let Some(pos) = cache.entries.iter().position(|(i, _)| *i == index) {
            let entry = cache.entries.remove(pos).ok_or("sprite-cache-error")?;
            let image = entry.1.clone();
            cache.entries.push_back(entry);
            return Ok(image);
        }
        let rgba = self
            .file
            .layer(index)
            .ok_or("sprite-layer-missing")?
            .rgba8()
            .map_err(|e| format!("sprite-psd-invalid: {e}"))?;
        let image = Arc::new(
            RgbaImage::from_raw(rgba.width, rgba.height, rgba.data).ok_or("sprite-psd-invalid")?,
        );
        let bytes = image.as_raw().len();
        while cache.bytes + bytes > IMAGE_CACHE_LIMIT {
            if let Some((_, old)) = cache.entries.pop_front() {
                cache.bytes -= old.as_raw().len();
            } else {
                break;
            }
        }
        if bytes <= IMAGE_CACHE_LIMIT {
            cache.bytes += bytes;
            cache.entries.push_back((index, image.clone()));
        }
        Ok(image)
    }
    pub fn visibility(
        &self,
        sprite: &SpriteRender,
        local: u64,
        timeline: u64,
        fps: FrameRate,
    ) -> Result<BTreeSet<String>> {
        let mut visible: BTreeSet<_> = self
            .all_nodes()
            .iter()
            .filter(|n| {
                sprite
                    .settings
                    .visibility
                    .get(&n.key)
                    .copied()
                    .unwrap_or(n.visible)
            })
            .map(|n| n.key.clone())
            .collect();
        let (mouth, eyes) = sprite.selected_layers(local, timeline, fps);
        for (animation, mut selected) in [
            (&sprite.settings.mouth, mouth),
            (&sprite.settings.blink, eyes),
        ] {
            // Empty selections mean unfinished settings, not an invisible face.
            // Use the explicit idle pose, or leave the base PSD/visibility overrides
            // intact when idle has not been configured (also while disabled).
            if selected.is_empty() {
                selected.extend(animation.idle_layers.iter().cloned());
            }
            if selected.is_empty() {
                continue;
            }
            let controlled = self.animation_scope(animation.keys())?;
            let selected = self.selected_animation_layers(&sprite.settings, &selected)?;
            visible.retain(|key| !controlled.contains(key));
            visible.extend(selected.iter().cloned());
        }
        Ok(visible)
    }
    pub fn composite(&self, visible: &BTreeSet<String>) -> Result<RgbaImage> {
        self.composite_cancellable(visible, &|| false)
    }
    /// Isolate a layer/group (including hidden variants) without changing the
    /// document or sprite settings. Descendants retain their PSD visibility.
    pub fn element_thumbnail(&self, key: &str, cancelled: &dyn Fn() -> bool) -> Result<RgbaImage> {
        fn descendants(node: &Node, visible: &mut BTreeSet<String>) {
            for child in &node.children {
                if child.visible {
                    visible.insert(child.key.clone());
                    descendants(child, visible);
                }
            }
        }
        fn find(nodes: &[Node], key: &str, visible: &mut BTreeSet<String>) -> bool {
            for node in nodes {
                if node.key == key {
                    visible.insert(node.key.clone());
                    descendants(node, visible);
                    return true;
                }
                if find(&node.children, key, visible) {
                    visible.insert(node.key.clone());
                    return true;
                }
            }
            false
        }
        if cancelled() {
            return Err("cancelled".into());
        }
        let mut visible = BTreeSet::new();
        if !find(&self.nodes, key, &mut visible) {
            return Err("sprite-layer-missing".into());
        }
        let image = self.composite_cancellable(&visible, cancelled)?;
        let mut bounds = [image.width(), image.height(), 0, 0];
        for (x, y, pixel) in image.enumerate_pixels() {
            if x == 0 && cancelled() {
                return Err("cancelled".into());
            }
            if pixel[3] > 0 {
                bounds[0] = bounds[0].min(x);
                bounds[1] = bounds[1].min(y);
                bounds[2] = bounds[2].max(x + 1);
                bounds[3] = bounds[3].max(y + 1);
            }
        }
        if bounds[2] == 0 {
            return Ok(RgbaImage::new(1, 1));
        }
        let view = image::imageops::crop_imm(
            &image,
            bounds[0],
            bounds[1],
            bounds[2] - bounds[0],
            bounds[3] - bounds[1],
        );
        let width = bounds[2] - bounds[0];
        let height = bounds[3] - bounds[1];
        let scale = (64.0 / width as f64).min(64.0 / height as f64).min(1.0);
        Ok(image::imageops::thumbnail(
            &*view,
            (width as f64 * scale).round().max(1.0) as u32,
            (height as f64 * scale).round().max(1.0) as u32,
        ))
    }
    pub fn composite_cancellable(
        &self,
        visible: &BTreeSet<String>,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<RgbaImage> {
        fn blend(dst: &mut RgbaImage, src: &RgbaImage, left: i32, top: i32, opacity: u8) {
            let x0 = left.max(0) as u32;
            let y0 = top.max(0) as u32;
            let x1 = (left as i64 + src.width() as i64).clamp(0, dst.width() as i64) as u32;
            let y1 = (top as i64 + src.height() as i64).clamp(0, dst.height() as i64) as u32;
            for y in y0..y1 {
                for x in x0..x1 {
                    let mut p = *src.get_pixel(
                        (x as i64 - left as i64) as u32,
                        (y as i64 - top as i64) as u32,
                    );
                    p[3] = ((p[3] as u32 * opacity as u32 + 127) / 255) as u8;
                    dst.get_pixel_mut(x, y).blend(&p);
                }
            }
        }
        fn draw(
            doc: &Document,
            nodes: &[Node],
            visible: &BTreeSet<String>,
            out: &mut RgbaImage,
            cancelled: &dyn Fn() -> bool,
        ) -> Result<()> {
            for n in nodes {
                if cancelled() {
                    return Err("cancelled".into());
                }
                if !visible.contains(&n.key) {
                    continue;
                }
                if n.group {
                    if n.pass_through {
                        draw(doc, &n.children, visible, out, cancelled)?;
                    } else {
                        let mut group = RgbaImage::new(doc.width, doc.height);
                        draw(doc, &n.children, visible, &mut group, cancelled)?;
                        blend(out, &group, 0, 0, n.opacity);
                    }
                } else if n.rect[2] > n.rect[0] && n.rect[3] > n.rect[1] {
                    let pixels = doc.layer_pixels(n.index)?;
                    let fill = doc.file.layers()[n.index].fill_opacity();
                    let opacity = ((n.opacity as u32 * fill as u32 + 127) / 255) as u8;
                    blend(out, &pixels, n.rect[0], n.rect[1], opacity);
                }
            }
            Ok(())
        }
        let mut out = RgbaImage::new(self.width, self.height);
        draw(self, &self.nodes, visible, &mut out, cancelled)?;
        Ok(out)
    }
}
