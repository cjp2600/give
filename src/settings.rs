//! Persisted application preferences. Missing fields inherit current defaults.
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    sync::{LazyLock, RwLock},
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FontFamily {
    #[default]
    SystemMono,
    Menlo,
    Monaco,
}
impl FontFamily {
    pub const ALL: [Self; 3] = [Self::SystemMono, Self::Menlo, Self::Monaco];
    pub fn name(self) -> &'static str {
        match self {
            Self::Menlo => "Menlo",
            Self::Monaco => "Monaco",
            Self::SystemMono => {
                #[cfg(target_os = "macos")]
                {
                    "Menlo"
                }
                #[cfg(windows)]
                {
                    "Consolas"
                }
                #[cfg(not(any(target_os = "macos", windows)))]
                {
                    "DejaVu Sans Mono"
                }
            }
        }
    }
    pub fn label(self) -> &'static str {
        if self == Self::SystemMono {
            "System monospace"
        } else {
            self.name()
        }
    }
}

/// What give starts through rcli when a project opens.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RcliAutostart {
    Off,
    Claude,
    Codex,
    #[default]
    Ask,
}
impl RcliAutostart {
    pub const ALL: [Self; 4] = [Self::Ask, Self::Claude, Self::Codex, Self::Off];
    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Claude => "Claude",
            Self::Codex => "Codex",
            Self::Ask => "Ask",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub theme: String,
    pub font_family: FontFamily,
    pub terminal_font_size: f32,
    pub preview_font_size: f32,
    pub terminal_min_columns: u16,
    pub cursor_blink: bool,
    pub refresh_seconds: u64,
    pub show_deletions: bool,
    pub recent_limit: usize,
    pub history_page_size: usize,
    /// The rcli binary; empty finds it on the login shell's PATH.
    pub rcli_path: String,
    pub rcli_autostart: RcliAutostart,
    /// Start agents with `-y`: no approval prompts.
    pub rcli_dangerous: bool,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: "give".into(),
            font_family: FontFamily::default(),
            terminal_font_size: 14.,
            preview_font_size: 15.,
            terminal_min_columns: 100,
            cursor_blink: true,
            refresh_seconds: 2,
            show_deletions: false,
            recent_limit: 10,
            history_page_size: 100,
            rcli_path: String::new(),
            rcli_autostart: RcliAutostart::default(),
            rcli_dangerous: false,
        }
    }
}
impl Settings {
    pub fn validate(&self) -> Result<()> {
        anyhow::ensure!(
            crate::theme::find(&self.theme).is_some(),
            "Unknown theme: {}",
            self.theme
        );
        anyhow::ensure!(
            (10. ..=28.).contains(&self.terminal_font_size),
            "Terminal font size must be between 10 and 28 px"
        );
        anyhow::ensure!(
            (10. ..=28.).contains(&self.preview_font_size),
            "Preview font size must be between 10 and 28 px"
        );
        anyhow::ensure!(
            (40..=200).contains(&self.terminal_min_columns),
            "Terminal width must be between 40 and 200 columns"
        );
        anyhow::ensure!(
            (1..=30).contains(&self.refresh_seconds),
            "Refresh interval must be between 1 and 30 seconds"
        );
        anyhow::ensure!(
            (5..=50).contains(&self.recent_limit),
            "Recent projects must be between 5 and 50"
        );
        anyhow::ensure!(
            (25..=500).contains(&self.history_page_size),
            "History page size must be between 25 and 500"
        );
        Ok(())
    }
    pub fn load(path: &Path) -> Result<Self> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(error) => return Err(error).context("Reading settings"),
        };
        let settings: Self = serde_json::from_str(&text).context("Reading settings JSON")?;
        settings.validate()?;
        Ok(settings)
    }
    pub fn save(&self, path: &Path) -> Result<()> {
        use std::io::Write;
        self.validate()?;
        let parent = path
            .parent()
            .context("Settings path needs a parent directory")?;
        std::fs::create_dir_all(parent)?;
        let mut file = tempfile::NamedTempFile::new_in(parent)?;
        file.write_all(serde_json::to_string_pretty(self)?.as_bytes())?;
        file.write_all(b"\n")?;
        file.as_file().sync_all()?;
        file.persist(path).context("Saving settings")?;
        Ok(())
    }
}
pub fn store() -> Option<PathBuf> {
    crate::recent::store().map(|p| p.with_file_name("settings.json"))
}
static ACTIVE: LazyLock<RwLock<Settings>> = LazyLock::new(|| RwLock::new(Settings::default()));
pub fn current() -> Settings {
    ACTIVE.read().unwrap_or_else(|e| e.into_inner()).clone()
}
pub fn set(settings: Settings) {
    *ACTIVE.write().unwrap_or_else(|e| e.into_inner()) = settings;
}
pub fn terminal_font_size() -> f32 {
    ACTIVE
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .terminal_font_size
}
pub fn terminal_line_height() -> f32 {
    terminal_font_size() * 17. / 14.
}
pub fn preview_line_height() -> f32 {
    ACTIVE
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .preview_font_size
        * 26.
        / 15.
}
pub fn font_family() -> &'static str {
    ACTIVE
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .font_family
        .name()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn defaults_migrate_partial_files_and_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("settings.json");
        assert_eq!(Settings::load(&file).unwrap(), Settings::default());
        std::fs::write(&file, r#"{"theme":"nord", "future_field":true}"#).unwrap();
        let mut settings = Settings::load(&file).unwrap();
        assert_eq!(settings.theme, "nord");
        assert_eq!(settings.terminal_font_size, 14.);
        settings.terminal_font_size = 18.;
        settings.save(&file).unwrap();
        assert_eq!(Settings::load(&file).unwrap(), settings);
    }
    #[test]
    fn invalid_values_never_overwrite_last_good_settings() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("settings.json");
        Settings::default().save(&file).unwrap();
        let invalid = Settings {
            refresh_seconds: 0,
            ..Settings::default()
        };
        assert!(invalid.save(&file).is_err());
        assert_eq!(Settings::load(&file).unwrap(), Settings::default());
        std::fs::write(&file, r#"{"theme":"missing"}"#).unwrap();
        assert!(Settings::load(&file).is_err());
        std::fs::write(&file, "broken JSON").unwrap();
        assert!(Settings::load(&file).is_err());
    }
}
