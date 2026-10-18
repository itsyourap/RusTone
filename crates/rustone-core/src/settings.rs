//! User settings, persisted as `settings.toml` in the config folder.
//!
//! Every field has a default, so a partial or older file loads and missing
//! keys take their defaults. Unknown keys are ignored.

use std::fmt;
use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

pub const FILE_NAME: &str = "settings.toml";

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub ui: UiSettings,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UiSettings {
    pub theme: ThemePreference,
    /// Draw RusTone's own title bar instead of the system frame.
    pub custom_titlebar: bool,
    /// Closing the window keeps RusTone running in the tray.
    pub close_to_tray: bool,
    /// Start in the tray without opening the window.
    pub start_minimized: bool,
    pub window: WindowGeometry,
    /// egui zoom factor (Ctrl +/-).
    pub zoom: f32,
}

impl Default for UiSettings {
    fn default() -> Self {
        Self {
            theme: ThemePreference::System,
            custom_titlebar: true,
            close_to_tray: true,
            start_minimized: false,
            window: WindowGeometry::default(),
            zoom: 1.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemePreference {
    /// Follow the operating system's light/dark appearance.
    #[default]
    System,
    Light,
    Dark,
}

/// The main window's last size and place, in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowGeometry {
    pub size: Option<[f32; 2]>,
    pub position: Option<[f32; 2]>,
    pub maximized: bool,
}

#[derive(Debug)]
pub enum LoadError {
    Io(io::Error),
    Parse(toml::de::Error),
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "cannot read settings: {e}"),
            Self::Parse(e) => write!(f, "settings file is invalid: {e}"),
        }
    }
}

impl std::error::Error for LoadError {}

impl Settings {
    /// Reads `path`. A missing file gives the defaults.
    pub fn load(path: &Path) -> Result<Self, LoadError> {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::from_toml(&text),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(LoadError::Io(e)),
        }
    }

    pub fn from_toml(text: &str) -> Result<Self, LoadError> {
        toml::from_str(text).map_err(LoadError::Parse)
    }

    pub fn to_toml(&self) -> String {
        toml::to_string_pretty(self).expect("settings always serialize")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let mut s = Settings::default();
        s.ui.theme = ThemePreference::Dark;
        s.ui.window = WindowGeometry { size: Some([400.0, 700.0]), position: Some([10.0, 20.0]), maximized: false };
        s.ui.zoom = 1.25;
        assert_eq!(Settings::from_toml(&s.to_toml()).unwrap(), s);
    }

    #[test]
    fn partial_file_takes_defaults() {
        let s = Settings::from_toml("[ui]\ntheme = \"light\"\nsomething_new = 1\n").unwrap();
        assert_eq!(s.ui.theme, ThemePreference::Light);
        assert!(s.ui.close_to_tray);
        assert_eq!(s.ui.zoom, 1.0);
    }

    #[test]
    fn missing_file_is_default() {
        let s = Settings::load(Path::new("definitely/not/here/settings.toml")).unwrap();
        assert_eq!(s, Settings::default());
    }

    #[test]
    fn invalid_file_is_an_error() {
        assert!(matches!(Settings::from_toml("[ui]\ntheme = 5"), Err(LoadError::Parse(_))));
    }
}
