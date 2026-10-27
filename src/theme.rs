//! Theme definitions shared by the desktop UI, syntax renderer and terminal.
use serde::Deserialize;
use std::sync::{
    Arc, LazyLock, RwLock,
    atomic::{AtomicU64, Ordering},
};

fn hex<'de, D: serde::Deserializer<'de>>(d: D) -> Result<u32, D::Error> {
    let value = String::deserialize(d)?;
    let digits = value
        .strip_prefix('#')
        .filter(|v| v.len() == 6)
        .ok_or_else(|| serde::de::Error::custom("expected #RRGGBB"))?;
    u32::from_str_radix(digits, 16).map_err(serde::de::Error::custom)
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Palette {
    #[serde(deserialize_with = "hex")]
    pub background: u32,
    #[serde(deserialize_with = "hex")]
    pub panel: u32,
    #[serde(deserialize_with = "hex")]
    pub surface: u32,
    #[serde(deserialize_with = "hex")]
    pub border: u32,
    #[serde(deserialize_with = "hex")]
    pub text: u32,
    #[serde(deserialize_with = "hex")]
    pub muted: u32,
    #[serde(deserialize_with = "hex")]
    pub accent: u32,
    #[serde(deserialize_with = "hex")]
    pub brand: u32,
    #[serde(deserialize_with = "hex")]
    pub green: u32,
    #[serde(deserialize_with = "hex")]
    pub red: u32,
    #[serde(deserialize_with = "hex")]
    pub yellow: u32,
    #[serde(deserialize_with = "hex")]
    pub modified: u32,
    #[serde(deserialize_with = "hex")]
    pub added: u32,
    #[serde(deserialize_with = "hex")]
    pub deleted: u32,
    #[serde(deserialize_with = "hex")]
    pub untracked: u32,
    #[serde(deserialize_with = "hex")]
    pub title_bar: u32,
    #[serde(deserialize_with = "hex")]
    pub selected: u32,
    #[serde(deserialize_with = "hex")]
    pub guide: u32,
    #[serde(deserialize_with = "hex")]
    pub scrollbar_track: u32,
    #[serde(deserialize_with = "hex")]
    pub scrollbar_thumb: u32,
    #[serde(deserialize_with = "hex")]
    pub preview_header: u32,
    #[serde(deserialize_with = "hex")]
    pub diff_added: u32,
    #[serde(deserialize_with = "hex")]
    pub diff_removed: u32,
    #[serde(deserialize_with = "hex")]
    pub cursor_line: u32,
    #[serde(deserialize_with = "hex")]
    pub selection: u32,
    #[serde(deserialize_with = "hex")]
    pub search_active: u32,
    #[serde(deserialize_with = "hex")]
    pub search_match: u32,
    #[serde(deserialize_with = "hex")]
    pub popup_border: u32,
    #[serde(deserialize_with = "hex")]
    pub folder: u32,
    #[serde(deserialize_with = "hex")]
    pub rust: u32,
    #[serde(deserialize_with = "hex")]
    pub go: u32,
    #[serde(deserialize_with = "hex")]
    pub markdown: u32,
    #[serde(deserialize_with = "hex")]
    pub config: u32,
    #[serde(deserialize_with = "hex")]
    pub terminal_background: u32,
    #[serde(deserialize_with = "hex")]
    pub terminal_foreground: u32,
    #[serde(deserialize_with = "hex")]
    pub terminal_selection: u32,
    #[serde(deserialize_with = "hex")]
    pub cursor: u32,
    #[serde(deserialize_with = "hex")]
    pub cursor_inactive: u32,
    #[serde(deserialize_with = "hex")]
    pub syntax_comment: u32,
    #[serde(deserialize_with = "hex")]
    pub syntax_keyword: u32,
    #[serde(deserialize_with = "hex")]
    pub syntax_string: u32,
    #[serde(deserialize_with = "hex")]
    pub syntax_number: u32,
    #[serde(deserialize_with = "hex")]
    pub syntax_type: u32,
    #[serde(deserialize_with = "hex")]
    pub syntax_variable: u32,
    #[serde(deserialize_with = "hex")]
    pub syntax_function: u32,
    #[serde(deserialize_with = "hex")]
    pub input: u32,
    #[serde(deserialize_with = "hex")]
    pub secondary: u32,
    #[serde(deserialize_with = "hex")]
    pub secondary_hover: u32,
    #[serde(deserialize_with = "hex")]
    pub secondary_active: u32,
    #[serde(deserialize_with = "hex")]
    pub primary: u32,
    #[serde(deserialize_with = "hex")]
    pub primary_hover: u32,
    #[serde(deserialize_with = "hex")]
    pub primary_active: u32,
    #[serde(deserialize_with = "hex")]
    pub primary_foreground: u32,
}

fn ansi<'de, D: serde::Deserializer<'de>>(d: D) -> Result<[u32; 16], D::Error> {
    let values = Vec::<String>::deserialize(d)?;
    let mut colors = Vec::new();
    for value in values {
        colors.push(hex(serde::de::value::StringDeserializer::<D::Error>::new(
            value,
        ))?);
    }
    colors
        .try_into()
        .map_err(|_| serde::de::Error::custom("expected 16 ANSI colors"))
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Theme {
    pub id: String,
    pub name: String,
    pub dark: bool,
    pub source: String,
    pub colors: Palette,
    #[serde(deserialize_with = "ansi")]
    pub ansi: [u32; 16],
}

pub static BUILTINS: LazyLock<Vec<Arc<Theme>>> = LazyLock::new(|| {
    [
        include_str!("../assets/themes/give.json"),
        include_str!("../assets/themes/nord.json"),
        include_str!("../assets/themes/dracula.json"),
        include_str!("../assets/themes/tokyo-night.json"),
        include_str!("../assets/themes/catppuccin-mocha.json"),
        include_str!("../assets/themes/catppuccin-latte.json"),
    ]
    .into_iter()
    .map(|json| Arc::new(serde_json::from_str(json).expect("valid bundled theme")))
    .collect()
});

static ACTIVE: LazyLock<RwLock<Arc<Theme>>> = LazyLock::new(|| RwLock::new(BUILTINS[0].clone()));
static REVISION: AtomicU64 = AtomicU64::new(0);

pub fn find(id: &str) -> Option<Arc<Theme>> {
    BUILTINS.iter().find(|theme| theme.id == id).cloned()
}
pub fn current() -> Arc<Theme> {
    ACTIVE.read().unwrap_or_else(|e| e.into_inner()).clone()
}
pub fn set(theme: Arc<Theme>) {
    *ACTIVE.write().unwrap_or_else(|e| e.into_inner()) = theme;
    REVISION.fetch_add(1, Ordering::Relaxed);
}
pub fn revision() -> u64 {
    REVISION.load(Ordering::Relaxed)
}

/// Syntax caches retain token roles, so changing theme never needs re-tokenization.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyntaxColor {
    Text,
    Comment,
    Keyword,
    String,
    Number,
    Type,
    Variable,
    Function,
}
impl SyntaxColor {
    pub fn resolve(self, p: &Palette) -> u32 {
        match self {
            Self::Text => p.text,
            Self::Comment => p.syntax_comment,
            Self::Keyword => p.syntax_keyword,
            Self::String => p.syntax_string,
            Self::Number => p.syntax_number,
            Self::Type => p.syntax_type,
            Self::Variable => p.syntax_variable,
            Self::Function => p.syntax_function,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn themes_are_complete_unique_and_tokens_recolor_without_reloading() {
        let mut ids = std::collections::HashSet::new();
        for theme in BUILTINS.iter() {
            assert!(ids.insert(&theme.id));
            assert_ne!(theme.colors.background, theme.colors.text);
            assert_ne!(theme.colors.diff_added, theme.colors.diff_removed);
            assert!(!theme.source.is_empty());
        }
        assert!(BUILTINS.iter().any(|t| !t.dark));
        let give = find("give").unwrap();
        let latte = find("catppuccin-latte").unwrap();
        assert_ne!(
            SyntaxColor::Keyword.resolve(&give.colors),
            SyntaxColor::Keyword.resolve(&latte.colors)
        );
    }
}
