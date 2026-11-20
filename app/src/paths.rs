//! Dossiers de l'app. Mêmes emplacements que l'ancienne version Tauri
//! (`app_config_dir` / `app_data_dir` = dossier système + identifiant), pour que
//! réglages et enregistrements existants soient repris tels quels.

use std::path::PathBuf;

pub const IDENTIFIER: &str = "com.pat.screenshotpp";
pub const APP_NAME: &str = "ScreenShotPP";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn config_dir() -> Result<PathBuf, String> {
    dirs::config_dir()
        .map(|d| d.join(IDENTIFIER))
        .ok_or_else(|| "Dossier de configuration introuvable".to_string())
}

pub fn data_dir() -> Result<PathBuf, String> {
    dirs::data_dir()
        .map(|d| d.join(IDENTIFIER))
        .ok_or_else(|| "Dossier de données introuvable".to_string())
}

pub fn desktop_dir() -> Option<PathBuf> {
    dirs::desktop_dir()
}
