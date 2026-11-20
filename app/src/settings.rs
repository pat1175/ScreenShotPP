// Le modèle de réglages pur (struct, défauts, lecture/écriture fichier) vit dans
// `screenshotpp-core` pour être testable sans l'UI. Ici : l'emplacement du fichier.
pub use screenshotpp_core::settings::{load_from_path, save_to_path, Settings};

/// Chemin du fichier de réglages dans le dossier de config de l'app.
pub fn settings_path() -> Result<std::path::PathBuf, String> {
    Ok(crate::paths::config_dir()?.join("settings.json"))
}

/// Charge les réglages de l'app (défauts si absent).
pub fn load() -> Settings {
    match settings_path() {
        Ok(p) => load_from_path(&p),
        Err(_) => Settings::default(),
    }
}

/// Sauvegarde les réglages de l'app.
pub fn save(settings: &Settings) -> Result<(), String> {
    save_to_path(&settings_path()?, settings)
}
