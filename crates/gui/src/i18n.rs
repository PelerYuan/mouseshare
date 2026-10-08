//! English + Simplified Chinese strings. Keys are looked up at runtime;
//! a missing key falls back to English and then to the key itself, and a
//! unit test (below) checks every key used in the source exists in both
//! languages, so a typo can never ship as raw text.

use mouseshare_config::Language;

use crate::strings::TABLE;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    En,
    ZhCn,
}

impl Lang {
    pub fn resolve(pref: Language, system: Lang) -> Lang {
        match pref {
            Language::Auto => system,
            Language::En => Lang::En,
            Language::ZhCn => Lang::ZhCn,
        }
    }

    /// Best guess at the desktop language from the locale environment.
    pub fn from_env() -> Lang {
        for var in ["LC_ALL", "LC_MESSAGES", "LANGUAGE", "LANG"] {
            if let Ok(v) = std::env::var(var) {
                if v.is_empty() {
                    continue;
                }
                return if v.to_ascii_lowercase().starts_with("zh") {
                    Lang::ZhCn
                } else {
                    Lang::En
                };
            }
        }
        Lang::En
    }
}

#[derive(Clone, Copy)]
pub struct Tr {
    pub lang: Lang,
}

impl Tr {
    pub fn new(lang: Lang) -> Self {
        Self { lang }
    }

    pub fn t(&self, key: &'static str) -> &'static str {
        match TABLE.iter().find(|(k, _, _)| *k == key) {
            Some((_, en, zh)) => match self.lang {
                Lang::En => en,
                Lang::ZhCn if !zh.is_empty() => zh,
                Lang::ZhCn => en,
            },
            None => key,
        }
    }

    /// Looks up `key` and substitutes `{0}`, `{1}`, ... with `args`.
    pub fn f(&self, key: &'static str, args: &[&str]) -> String {
        let mut s = self.t(key).to_string();
        for (i, a) in args.iter().enumerate() {
            s = s.replace(&format!("{{{i}}}"), a);
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOURCES: &[&str] = &[
        include_str!("app.rs"),
        include_str!("canvas.rs"),
        include_str!("rail.rs"),
        include_str!("dialogs.rs"),
        include_str!("target_view.rs"),
        include_str!("settings_view.rs"),
        include_str!("onboarding.rs"),
        include_str!("widgets.rs"),
    ];

    /// Every `.t("key")` / `.f("key", ..)` in the sources (the key may sit
    /// on the line after the opening parenthesis).
    fn used_keys() -> Vec<String> {
        let mut keys = Vec::new();
        for src in SOURCES {
            for pat in [".t(", ".f("] {
                let mut rest = *src;
                while let Some(i) = rest.find(pat) {
                    rest = rest[i + pat.len()..].trim_start();
                    if let Some(body) = rest.strip_prefix('"') {
                        if let Some(end) = body.find('"') {
                            keys.push(body[..end].to_string());
                        }
                    }
                }
            }
        }
        keys
    }

    #[test]
    fn every_used_key_is_translated() {
        let keys = used_keys();
        assert!(
            keys.len() > 50,
            "scanner found too few keys: {}",
            keys.len()
        );
        for k in keys {
            let entry = TABLE.iter().find(|(key, _, _)| *key == k);
            let (_, en, zh) = entry.unwrap_or_else(|| panic!("missing string key {k:?}"));
            assert!(!en.is_empty(), "{k}: empty English");
            assert!(!zh.is_empty(), "{k}: empty Chinese");
        }
    }

    #[test]
    fn table_has_no_duplicates_and_matching_placeholders() {
        let mut seen = std::collections::HashSet::new();
        for (k, en, zh) in TABLE {
            assert!(seen.insert(*k), "duplicate key {k}");
            for i in 0..4 {
                let ph = format!("{{{i}}}");
                assert_eq!(
                    en.contains(&ph),
                    zh.contains(&ph),
                    "{k}: placeholder {ph} mismatch"
                );
            }
        }
    }
}
