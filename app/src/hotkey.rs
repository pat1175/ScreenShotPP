//! Raccourcis globaux (crate `global-hotkey`, celle qu'utilisait le plugin Tauri).
//! Les appuis arrivent dans la boucle principale via `UserEvent::Hotkey`.

use global_hotkey::hotkey::HotKey;
use global_hotkey::GlobalHotKeyManager;
use std::collections::HashMap;
use std::str::FromStr;

use crate::settings::Settings;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HotkeyAction {
    Capture,
    DelayedCapture,
    Record,
    CancelCountdown,
}

pub struct Hotkeys {
    manager: GlobalHotKeyManager,
    registered: Vec<HotKey>,
    cancel: Option<HotKey>,
    actions: HashMap<u32, HotkeyAction>,
}

fn parse(accelerator: &str) -> Result<HotKey, String> {
    HotKey::from_str(accelerator).map_err(|e| format!("Raccourci invalide {accelerator}: {e}"))
}

impl Hotkeys {
    pub fn new() -> Result<Self, String> {
        Ok(Hotkeys {
            manager: GlobalHotKeyManager::new().map_err(|e| e.to_string())?,
            registered: Vec::new(),
            cancel: None,
            actions: HashMap::new(),
        })
    }

    pub fn action(&self, id: u32) -> Option<HotkeyAction> {
        self.actions.get(&id).copied()
    }

    fn unregister_main(&mut self) {
        for hk in self.registered.drain(..) {
            let _ = self.manager.unregister(hk);
            self.actions.remove(&hk.id());
        }
    }

    /// Désenregistre tout puis enregistre les raccourcis instantané, différé et vidéo.
    pub fn register_all(&mut self, settings: &Settings) -> Result<(), String> {
        self.unregister_main();
        let wanted = [
            (&settings.capture_shortcut, HotkeyAction::Capture),
            (
                &settings.delayed_capture_shortcut,
                HotkeyAction::DelayedCapture,
            ),
            (&settings.record_shortcut, HotkeyAction::Record),
        ];
        for (accelerator, action) in wanted {
            let hk = parse(accelerator)?;
            self.manager
                .register(hk)
                .map_err(|e| format!("Échec d'enregistrement du raccourci {accelerator}: {e}"))?;
            self.registered.push(hk);
            self.actions.insert(hk.id(), action);
        }
        Ok(())
    }

    /// Raccourci d'annulation, actif seulement pendant un compte à rebours.
    pub fn register_cancel(&mut self, accelerator: &str) -> Result<(), String> {
        self.unregister_cancel();
        let hk = parse(accelerator)?;
        self.manager.register(hk).map_err(|e| e.to_string())?;
        self.actions.insert(hk.id(), HotkeyAction::CancelCountdown);
        self.cancel = Some(hk);
        Ok(())
    }

    pub fn unregister_cancel(&mut self) {
        if let Some(hk) = self.cancel.take() {
            let _ = self.manager.unregister(hk);
            // Ne retire l'action que si aucun raccourci principal ne partage l'id.
            if !self.registered.iter().any(|r| r.id() == hk.id()) {
                self.actions.remove(&hk.id());
            }
        }
    }
}

/// Construit un accélérateur ("CmdOrCtrl+Shift+2") à partir d'un appui clavier
/// egui, ou `None` si la touche n'est pas utilisable seule comme raccourci.
/// Même convention que l'ancien enregistreur : ⌘ → CmdOrCtrl, Ctrl seul → Ctrl.
pub fn accelerator_from_key(key: egui::Key, modifiers: egui::Modifiers) -> Option<String> {
    let main = key_name(key)?;
    let mut parts: Vec<&str> = Vec::new();
    if modifiers.mac_cmd {
        parts.push("CmdOrCtrl");
    }
    if modifiers.ctrl && !modifiers.mac_cmd {
        parts.push("Ctrl");
    }
    if modifiers.alt {
        parts.push("Alt");
    }
    if modifiers.shift {
        parts.push("Shift");
    }
    parts.push(main);
    Some(parts.join("+"))
}

fn key_name(key: egui::Key) -> Option<&'static str> {
    use egui::Key::*;
    Some(match key {
        Space => "Space",
        ArrowUp => "Up",
        ArrowDown => "Down",
        ArrowLeft => "Left",
        ArrowRight => "Right",
        Escape => "Escape",
        Enter => "Enter",
        Tab => "Tab",
        Num0 => "0",
        Num1 => "1",
        Num2 => "2",
        Num3 => "3",
        Num4 => "4",
        Num5 => "5",
        Num6 => "6",
        Num7 => "7",
        Num8 => "8",
        Num9 => "9",
        F1 => "F1",
        F2 => "F2",
        F3 => "F3",
        F4 => "F4",
        F5 => "F5",
        F6 => "F6",
        F7 => "F7",
        F8 => "F8",
        F9 => "F9",
        F10 => "F10",
        F11 => "F11",
        F12 => "F12",
        _ => {
            let name = key.name();
            // Lettres A-Z : `Key::name` renvoie déjà la majuscule.
            if name.len() == 1 && name.as_bytes()[0].is_ascii_uppercase() {
                name
            } else {
                return None;
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Key, Modifiers};

    fn mods(mac_cmd: bool, ctrl: bool, alt: bool, shift: bool) -> Modifiers {
        Modifiers {
            alt,
            ctrl,
            shift,
            mac_cmd,
            command: mac_cmd || ctrl,
        }
    }

    #[test]
    fn command_shift_digit_maps_to_cmd_or_ctrl() {
        assert_eq!(
            accelerator_from_key(Key::Num2, mods(true, false, false, true)).as_deref(),
            Some("CmdOrCtrl+Shift+2")
        );
    }

    #[test]
    fn plain_ctrl_stays_ctrl_and_letters_are_uppercase() {
        assert_eq!(
            accelerator_from_key(Key::A, mods(false, true, true, false)).as_deref(),
            Some("Ctrl+Alt+A")
        );
    }

    #[test]
    fn named_keys_use_accelerator_names() {
        assert_eq!(
            accelerator_from_key(Key::ArrowUp, mods(false, true, false, false)).as_deref(),
            Some("Ctrl+Up")
        );
        assert_eq!(
            accelerator_from_key(Key::Escape, Modifiers::NONE).as_deref(),
            Some("Escape")
        );
    }

    #[test]
    fn unsupported_keys_yield_none() {
        assert_eq!(accelerator_from_key(Key::Backspace, Modifiers::CTRL), None);
    }

    #[test]
    fn every_produced_accelerator_parses_as_a_global_hotkey() {
        for (key, m) in [
            (Key::Num2, mods(true, false, false, true)),
            (Key::A, mods(false, true, true, false)),
            (Key::F5, mods(false, true, false, false)),
            (Key::Space, mods(false, true, false, true)),
            (Key::Escape, Modifiers::NONE),
        ] {
            let acc = accelerator_from_key(key, m).unwrap();
            assert!(parse(&acc).is_ok(), "{acc} ne se parse pas");
        }
    }
}
