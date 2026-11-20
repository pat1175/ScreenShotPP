//! Icône de zone de notification / barre des menus (crate `tray-icon`).

use tray_icon::menu::{Menu, MenuEvent, MenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

const SETTINGS: &str = "settings";
const STOP: &str = "stop-recording";
const QUIT: &str = "quit";

pub enum MenuAction {
    Settings,
    StopRecording,
    Quit,
}

pub fn menu_action(event: &MenuEvent) -> Option<MenuAction> {
    match event.id().as_ref() {
        SETTINGS => Some(MenuAction::Settings),
        STOP => Some(MenuAction::StopRecording),
        QUIT => Some(MenuAction::Quit),
        _ => None,
    }
}

pub struct Tray {
    icon: TrayIcon,
}

fn app_icon() -> Result<Icon, String> {
    let img = crate::icon::app_icon_rgba();
    Icon::from_rgba(img.as_raw().clone(), img.width(), img.height()).map_err(|e| e.to_string())
}

/// Menu normal (réglages, quitter) ou d'enregistrement (stop, quitter).
fn menu(recording: bool) -> Result<Menu, String> {
    let first = if recording {
        MenuItem::with_id(STOP, "Stop recording", true, None)
    } else {
        MenuItem::with_id(SETTINGS, "Open settings", true, None)
    };
    let quit = MenuItem::with_id(QUIT, "Quit", true, None);
    Menu::with_items(&[&first, &quit]).map_err(|e| e.to_string())
}

impl Tray {
    pub fn new() -> Result<Self, String> {
        let icon = TrayIconBuilder::new()
            .with_icon(app_icon()?)
            .with_tooltip(crate::paths::APP_NAME)
            .with_menu(Box::new(menu(false)?))
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Tray { icon })
    }

    /// Pastille rouge + item "Stop recording" pendant l'enregistrement.
    pub fn set_recording(&mut self, on: bool) {
        let icon = if on {
            let size = 32u32;
            let rgba = screenshotpp_core::record::recording_badge_rgba(size);
            Icon::from_rgba(rgba, size, size).map_err(|e| e.to_string())
        } else {
            app_icon()
        };
        if let Ok(icon) = icon {
            let _ = self.icon.set_icon(Some(icon));
        }
        if let Ok(m) = menu(on) {
            self.icon.set_menu(Some(Box::new(m)));
        }
    }
}
