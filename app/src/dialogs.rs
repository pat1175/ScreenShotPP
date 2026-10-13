//! Boîtes de dialogue natives (rfd). Bloquantes : appelées depuis le thread
//! principal, en réponse à une action utilisateur.

use rfd::{MessageButtons, MessageDialog, MessageDialogResult, MessageLevel};
use winit::window::Window;

pub fn error(parent: Option<&Window>, message: &str) {
    let mut d = MessageDialog::new()
        .set_level(MessageLevel::Error)
        .set_title(crate::paths::APP_NAME)
        .set_description(message)
        .set_buttons(MessageButtons::Ok);
    if let Some(p) = parent {
        d = d.set_parent(p);
    }
    d.show();
}

/// Confirmation Oui/Non (avertissement). `true` si l'utilisateur confirme.
pub fn confirm(parent: Option<&Window>, title: &str, message: &str) -> bool {
    let mut d = MessageDialog::new()
        .set_level(MessageLevel::Warning)
        .set_title(title)
        .set_description(message)
        .set_buttons(MessageButtons::OkCancel);
    if let Some(p) = parent {
        d = d.set_parent(p);
    }
    matches!(d.show(), MessageDialogResult::Ok | MessageDialogResult::Yes)
}
