// Pas de console sous Windows en release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod autostart;
mod capture;
mod capture_flow;
#[cfg(windows)]
mod capture_win;
mod clipboard;
mod dialogs;
mod gl_window;
mod hotkey;
mod icon;
mod ocr;
mod paths;
mod platform;
mod record;
mod settings;
mod storage;
#[cfg(any(windows, target_os = "macos"))]
mod tray;
mod ui;
mod updater;
mod window_pick;

fn main() {
    app::run();
}
