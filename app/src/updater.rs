//! Mises à jour : même manifeste `latest.json` et mêmes signatures minisign que
//! l'ancien plugin updater Tauri (clé publique inchangée), donc compatible avec
//! les releases existantes.

use crate::app::{Command, UserEvent};
use winit::event_loop::EventLoopProxy;

pub const RELEASES_URL: &str = "https://github.com/xVc323/ScreenShotPP/releases";
#[cfg(any(windows, target_os = "macos"))]
const ENDPOINT: &str =
    "https://github.com/xVc323/ScreenShotPP/releases/latest/download/latest.json";
#[cfg(any(windows, target_os = "macos"))]
const PUBKEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IEVBRjNBODg2NkY4MEIyQTgKUldTb3NvQnZocWp6NnZFMFpadDA4dFpGT1ViK1pQRnlpbVd3bEVIcWhOaGJ5aW1ZTDJFTjM4ck8K";

#[derive(Clone, Debug)]
pub struct UpdateInfo {
    pub version: String,
    #[allow(dead_code)] // affiché plus tard ; gardé pour le manifeste complet
    pub notes: String,
    #[cfg_attr(not(any(windows, target_os = "macos")), allow(dead_code))]
    url: String,
    #[cfg_attr(not(any(windows, target_os = "macos")), allow(dead_code))]
    signature: String,
}

/// Clé de plateforme du manifeste (convention Tauri).
#[cfg(any(windows, target_os = "macos"))]
fn platform_key() -> &'static str {
    if cfg!(windows) {
        "windows-x86_64"
    } else if cfg!(target_arch = "aarch64") {
        "darwin-aarch64"
    } else {
        "darwin-x86_64"
    }
}

/// Extrait la mise à jour d'un manifeste si elle est plus récente que `current`.
pub fn parse_manifest(
    json: &str,
    platform: &str,
    current: &str,
) -> Result<Option<UpdateInfo>, String> {
    let v: serde_json::Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
    let version = v["version"]
        .as_str()
        .ok_or("Manifeste sans version")?
        .trim_start_matches('v')
        .to_string();
    let remote = semver::Version::parse(&version).map_err(|e| e.to_string())?;
    let local = semver::Version::parse(current).map_err(|e| e.to_string())?;
    if remote <= local {
        return Ok(None);
    }
    let p = &v["platforms"][platform];
    let url = p["url"].as_str().ok_or("Plateforme absente du manifeste")?;
    let signature = p["signature"].as_str().ok_or("Signature absente")?;
    Ok(Some(UpdateInfo {
        version,
        notes: v["notes"].as_str().unwrap_or_default().to_string(),
        url: url.to_string(),
        signature: signature.to_string(),
    }))
}

#[cfg(any(windows, target_os = "macos"))]
pub fn check() -> Result<Option<UpdateInfo>, String> {
    let body = ureq::get(ENDPOINT)
        .call()
        .map_err(|e| e.to_string())?
        .body_mut()
        .read_to_string()
        .map_err(|e| e.to_string())?;
    parse_manifest(&body, platform_key(), crate::paths::VERSION)
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn check() -> Result<Option<UpdateInfo>, String> {
    Err("Mises à jour non prises en charge sur cette plateforme".into())
}

/// Vérification silencieuse au démarrage : seule une mise à jour disponible
/// remonte (la bannière s'affiche alors dans les réglages).
pub fn check_in_background(proxy: EventLoopProxy<UserEvent>) {
    std::thread::spawn(move || {
        if let Ok(Some(info)) = check() {
            let _ = proxy.send_event(UserEvent::Command(Command::UpdateAvailable(info)));
        }
    });
}

/// Vérifie une signature minisign au format Tauri (fichier `.sig` et clé publique
/// encodés en base64 autour du texte minisign).
#[cfg(any(windows, target_os = "macos"))]
pub fn verify_signature(data: &[u8], signature_b64: &str, pubkey_b64: &str) -> Result<(), String> {
    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD;
    let pk_text = String::from_utf8(b64.decode(pubkey_b64.trim()).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    let sig_text = String::from_utf8(
        b64.decode(signature_b64.trim())
            .map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let pk = minisign_verify::PublicKey::decode(&pk_text).map_err(|e| e.to_string())?;
    let sig = minisign_verify::Signature::decode(&sig_text).map_err(|e| e.to_string())?;
    pk.verify(data, &sig, false)
        .map_err(|e| format!("Signature de mise à jour invalide: {e}"))
}

/// Télécharge, vérifie et installe. `progress(reçu, total)`. Au retour `Ok`,
/// l'app doit se terminer (Windows : l'installeur prend le relais ; macOS : la
/// nouvelle version a été relancée).
#[cfg(any(windows, target_os = "macos"))]
pub fn download_and_install(
    info: &UpdateInfo,
    progress: impl Fn(u64, Option<u64>),
) -> Result<(), String> {
    use std::io::Read;
    let mut response = ureq::get(&info.url).call().map_err(|e| e.to_string())?;
    let total = response
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok());
    let mut reader = response.body_mut().as_reader();
    let mut data = Vec::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = reader.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        data.extend_from_slice(&buf[..n]);
        progress(data.len() as u64, total);
    }
    verify_signature(&data, &info.signature, PUBKEY)?;
    install(&data)
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn download_and_install(
    _info: &UpdateInfo,
    _progress: impl Fn(u64, Option<u64>),
) -> Result<(), String> {
    Err("Mises à jour non prises en charge sur cette plateforme".into())
}

/// Windows : lance l'installeur NSIS téléchargé (il remplace l'app installée).
#[cfg(windows)]
fn install(data: &[u8]) -> Result<(), String> {
    let path = std::env::temp_dir().join("ScreenShotPP-update-setup.exe");
    std::fs::write(&path, data).map_err(|e| e.to_string())?;
    std::process::Command::new(&path)
        .spawn()
        .map_err(|e| format!("Lancement de l'installeur échoué: {e}"))?;
    Ok(())
}

/// macOS : extrait l'archive `.app.tar.gz` à côté du bundle courant, permute,
/// puis relance la nouvelle version.
#[cfg(target_os = "macos")]
fn install(data: &[u8]) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let bundle = exe
        .ancestors()
        .find(|p| p.extension().is_some_and(|e| e == "app"))
        .ok_or("L'app ne tourne pas depuis un bundle .app")?
        .to_path_buf();
    let parent = bundle.parent().ok_or("Bundle sans dossier parent")?;
    let staging = parent.join(".screenshotpp-update");
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging).map_err(|e| e.to_string())?;
    tar::Archive::new(flate2::read::GzDecoder::new(data))
        .unpack(&staging)
        .map_err(|e| format!("Archive de mise à jour illisible: {e}"))?;
    let new_app = std::fs::read_dir(&staging)
        .map_err(|e| e.to_string())?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|e| e == "app"))
        .ok_or("Archive sans bundle .app")?;
    let backup = parent.join(".screenshotpp-previous.app");
    let _ = std::fs::remove_dir_all(&backup);
    std::fs::rename(&bundle, &backup).map_err(|e| e.to_string())?;
    if let Err(e) = std::fs::rename(&new_app, &bundle) {
        let _ = std::fs::rename(&backup, &bundle);
        return Err(e.to_string());
    }
    let _ = std::fs::remove_dir_all(&backup);
    let _ = std::fs::remove_dir_all(&staging);
    std::process::Command::new("open")
        .arg("-n")
        .arg(&bundle)
        .spawn()
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const MANIFEST: &str = r#"{
        "version": "0.6.0",
        "notes": "Notes",
        "platforms": {
            "windows-x86_64": { "signature": "c2ln", "url": "https://x/setup.exe" },
            "darwin-aarch64": { "signature": "c2ln", "url": "https://x/app.tar.gz" }
        }
    }"#;

    #[test]
    fn newer_version_is_offered_for_the_current_platform() {
        let info = parse_manifest(MANIFEST, "windows-x86_64", "0.5.1")
            .unwrap()
            .unwrap();
        assert_eq!(info.version, "0.6.0");
        assert_eq!(info.url, "https://x/setup.exe");
    }

    #[test]
    fn same_or_older_version_is_not_offered() {
        assert!(parse_manifest(MANIFEST, "windows-x86_64", "0.6.0")
            .unwrap()
            .is_none());
        assert!(parse_manifest(MANIFEST, "windows-x86_64", "0.7.0")
            .unwrap()
            .is_none());
    }

    #[test]
    fn prerelease_is_older_than_its_release() {
        assert!(parse_manifest(MANIFEST, "darwin-aarch64", "0.6.0-rc.1")
            .unwrap()
            .is_some());
    }

    #[test]
    fn missing_platform_is_an_error() {
        assert!(parse_manifest(MANIFEST, "darwin-x86_64", "0.5.1").is_err());
    }
}
