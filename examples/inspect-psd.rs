//! Read-only diagnostics for a user-supplied PSD. No external artwork is bundled.
use apeireth_image_psd::PsdFile;
use std::{env, fs, path::PathBuf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = PathBuf::from(env::args_os().nth(1).ok_or("PSD path required")?);
    if fs::metadata(&path)?.len() > 64 * 1024 * 1024 {
        return Err("PSD exceeds diagnostic file limit".into());
    }
    let file = PsdFile::from_bytes(&fs::read(&path)?)?;
    println!("header={:?}; records={}", file.header, file.layers().len());
    for (index, record) in file.layers().iter().enumerate() {
        println!(
            "{index}: {:?} {:?} {:?} visible={} opacity={} clipping={} mask={} flags={:?} rect={:?}",
            record.name(),
            record.section_type(),
            record.blend_mode,
            record.is_visible(),
            record.opacity,
            record.clipping,
            record.layer_mask().is_some(),
            record.flags,
            record.rect
        );
    }
    match voxmoviemaker::sprites::psd::load(&path) {
        Ok(doc) => {
            println!(
                "VMM nodes={} roots={}",
                doc.all_nodes().len(),
                doc.nodes.len()
            );
            for node in &doc.nodes {
                println!("ROOT {} children={}", node.name, node.children.len());
            }
            if env::args().any(|a| a == "--render") {
                // Derived diagnostic images go to a new private temporary folder,
                // never the repository or the source artwork directory.
                use std::os::unix::fs::DirBuilderExt;
                let output =
                    env::temp_dir().join(format!("vmm-psd-review-{}", uuid::Uuid::new_v4()));
                let resolved = voxmoviemaker::assets::resolved_path(&output)?;
                let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR")).canonicalize()?;
                if resolved.starts_with(repository)
                    || resolved
                        .starts_with(path.canonicalize()?.parent().ok_or("PSD parent required")?)
                {
                    return Err(
                        "diagnostic output must be outside repository and source directory".into(),
                    );
                }
                fs::DirBuilder::new().mode(0o700).create(&output)?;
                let merged = file.composite_rgba8()?;
                let merged = image::RgbaImage::from_raw(merged.width, merged.height, merged.data)
                    .ok_or("invalid merged image")?;
                merged.save(output.join("merged.png"))?;
                let visible = doc
                    .all_nodes()
                    .iter()
                    .filter(|n| n.visible)
                    .map(|n| n.key.clone())
                    .collect();
                let rendered = doc.composite(&visible)?;
                rendered.save(output.join("vmm.png"))?;
                let mismatch = merged
                    .pixels()
                    .zip(rendered.pixels())
                    .filter(|(a, b)| {
                        (a[3] as i32 - b[3] as i32).abs() > 4
                            || (0..3).any(|i| {
                                (a[i] as i32 * a[3] as i32 - b[i] as i32 * b[3] as i32).abs()
                                    > 4 * 255
                            })
                    })
                    .count();
                println!(
                    "merged mismatched visible pixels={mismatch}/{}",
                    merged.width() as usize * merged.height() as usize
                );
                let nodes = doc.all_nodes();
                let mut sheet = image::RgbaImage::from_pixel(
                    8 * 80,
                    nodes.len().div_ceil(8) as u32 * 80,
                    image::Rgba([190, 190, 190, 255]),
                );
                for (i, node) in nodes.iter().enumerate() {
                    match doc.element_thumbnail(&node.key, &|| false) {
                        Ok(thumb) => {
                            let x = (i % 8) as i64 * 80 + 8;
                            let y = (i / 8) as i64 * 80 + 8;
                            image::imageops::overlay(&mut sheet, &thumb, x, y);
                            println!(
                                "TILE {i} {} {}x{}",
                                node.name,
                                thumb.width(),
                                thumb.height()
                            );
                        }
                        Err(e) => println!("TILE {i} ERROR {}: {e}", node.name),
                    }
                }
                sheet.save(output.join("layers.png"))?;
                println!("OUTPUT {}", output.display());
            }
        }
        Err(error) => println!("VMM error: {error}"),
    }
    Ok(())
}
