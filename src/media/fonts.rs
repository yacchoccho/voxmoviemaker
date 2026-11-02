//! List installed fonts via the same system Fontconfig tools used by font fallback.
use crate::{Result, media::ffmpeg};
use std::{collections::HashSet, path::PathBuf, process::Command, time::Duration};

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct FontId {
    pub path: PathBuf,
    pub index: u32,
}
#[derive(Debug)]
pub struct SystemFont {
    pub id: FontId,
    pub name: String,
}
pub fn list() -> Result<Vec<SystemFont>> {
    let output = ffmpeg::run(
        Command::new("fc-list").args(["--format", "%{file}\t%{index}\t%{family}\t%{style}\n"]),
        Duration::from_secs(5),
        || false,
    )?;
    let text = String::from_utf8(output.stdout).map_err(|e| e.to_string())?;
    let fonts = parse(&text);
    if fonts.is_empty() {
        return Err("font-list-empty".into());
    }
    Ok(fonts)
}
fn parse(text: &str) -> Vec<SystemFont> {
    let mut seen = HashSet::new();
    let mut fonts: Vec<_> = text
        .lines()
        .filter_map(|line| {
            let mut fields = line.splitn(4, '\t');
            let path = PathBuf::from(fields.next()?);
            let index: u32 = fields.next()?.parse().ok()?;
            let family = fields.next()?.trim();
            let style = fields.next()?.trim();
            let ext = path.extension()?.to_str()?.to_ascii_lowercase();
            // Keep the collection-index limits and font formats of the existing renderer.
            if index > 64
                || !matches!(ext.as_str(), "ttf" | "otf" | "ttc" | "otc")
                || family.is_empty()
                || !path.is_absolute()
            {
                return None;
            }
            let id = FontId { path, index };
            if !seen.insert(id.clone()) {
                return None;
            }
            Some(SystemFont {
                id,
                name: if style.is_empty() {
                    family.into()
                } else {
                    format!("{family} / {style}")
                },
            })
        })
        .collect();
    fonts.sort_by_cached_key(|f| (f.name.to_lowercase(), f.id.path.clone(), f.id.index));
    fonts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires installed system Fontconfig and fonts"]
    fn installed_font_list_contains_existing_files_and_valid_collection_indices() {
        let fonts = list().unwrap();
        assert!(!fonts.is_empty());
        for font in &fonts {
            assert!(font.id.path.is_file(), "{}", font.id.path.display());
            assert!(font.id.index <= 64);
            assert!(!font.name.is_empty());
        }
        println!("{} installed font faces listed", fonts.len());
    }

    #[test]
    fn fontconfig_names_paths_and_collection_indices_are_preserved() {
        let fonts = parse(concat!(
            "/usr/share/fonts/日本語.ttc\t2\t日本語フォント\tRegular\n",
            "/usr/share/fonts/日本語.ttc\t2\tduplicate\tRegular\n",
            "/usr/share/fonts/font.ttf\t0\tAlpha\tBold\n",
            "/usr/share/fonts/variable.ttf\t65536\tUnsupported instance\t\n",
            "/usr/share/fonts/bitmap.pcf\t0\tBitmap\t\n",
            "invalid\n",
        ));
        assert_eq!(fonts.len(), 2);
        assert_eq!(fonts[0].name, "Alpha / Bold");
        assert_eq!(fonts[1].id.index, 2);
        assert_eq!(
            fonts[1].id.path,
            PathBuf::from("/usr/share/fonts/日本語.ttc")
        );
    }
}
