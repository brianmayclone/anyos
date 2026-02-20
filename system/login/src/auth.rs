//! Authentication logic.

use anyos_std::process;
use uisys_client::UiTextField;

/// Try to authenticate. Returns the uid on success, or u32::MAX on failure.
pub fn try_login(username_field: &UiTextField, password_field: &UiTextField) -> u32 {
    let username = username_field.text();
    let password = password_field.text();

    if username.is_empty() {
        return u32::MAX;
    }

    if process::authenticate(username, password) {
        process::getuid() as u32
    } else {
        u32::MAX
    }
}
