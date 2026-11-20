//! Lancement à l'ouverture de session (crate `auto-launch`, celle du plugin
//! Tauri : LaunchAgent sur macOS, clé Run du registre sur Windows).

fn launcher() -> Result<auto_launch::AutoLaunch, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut builder = auto_launch::AutoLaunchBuilder::new();
    builder
        .set_app_name(crate::paths::APP_NAME)
        .set_app_path(&exe.to_string_lossy());
    #[cfg(target_os = "macos")]
    builder.set_macos_launch_mode(auto_launch::MacOSLaunchMode::LaunchAgent);
    builder.build().map_err(|e| e.to_string())
}

pub fn set_enabled(enabled: bool) -> Result<(), String> {
    let l = launcher()?;
    let result = if enabled {
        l.enable()
    } else if l.is_enabled().unwrap_or(false) {
        l.disable()
    } else {
        Ok(())
    };
    result.map_err(|e| format!("Lancement à l'ouverture de session: {e}"))
}
