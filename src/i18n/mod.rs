use fluent_bundle::{FluentBundle, FluentResource};
use std::{collections::BTreeMap, fs, path::Path};
use unic_langid::LanguageIdentifier;

pub struct I18n {
    bundles: BTreeMap<String, FluentBundle<FluentResource>>,
    pub language: String,
}
impl Default for I18n {
    fn default() -> Self {
        let mut result = Self {
            bundles: BTreeMap::new(),
            language: "ja-JP".into(),
        };
        result.add("en-US", include_str!("../../locales/en-US/main.ftl"));
        result.add("ja-JP", include_str!("../../locales/ja-JP/main.ftl"));
        result.add("zh-CN", include_str!("../../locales/zh-CN/main.ftl"));
        result
    }
}
impl I18n {
    pub fn add(&mut self, language: &str, source: &str) -> bool {
        let Ok(id) = language.parse::<LanguageIdentifier>() else {
            return false;
        };
        let Ok(resource) = FluentResource::try_new(source.into()) else {
            return false;
        };
        let mut bundle = FluentBundle::new(vec![id]);
        if bundle.add_resource(resource).is_err() {
            return false;
        }
        self.bundles.insert(language.into(), bundle);
        true
    }
    pub fn load_directory(&mut self, directory: &Path) {
        let Ok(entries) = fs::read_dir(directory) else {
            return;
        };
        for entry in entries.flatten().take(64) {
            if let (Some(language), Ok(source)) = (
                entry.file_name().to_str(),
                crate::project::persistence::read_limited(
                    &entry.path().join("main.ftl"),
                    1024 * 1024,
                ),
            ) && source.len() <= 1024 * 1024
            {
                self.add(language, &source);
            }
        }
    }
    pub fn languages(&self) -> Vec<String> {
        self.bundles.keys().cloned().collect()
    }
    pub fn text(&self, key: &str) -> String {
        for language in [self.language.as_str(), "en-US"] {
            if let Some(bundle) = self.bundles.get(language)
                && let Some(message) = bundle.get_message(key)
                && let Some(pattern) = message.value()
            {
                return bundle
                    .format_pattern(pattern, None, &mut Vec::new())
                    .into_owned();
            }
        }
        key.into()
    }
}
