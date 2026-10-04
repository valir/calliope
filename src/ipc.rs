//! Tauri commands: thin wrappers over pure modules, registered in `gui.rs`.

use crate::settings::{Settings, SettingsStore, Theme};

#[tauri::command]
pub fn app_version() -> String {
    crate::VERSION.to_string()
}

#[tauri::command]
pub fn get_settings(store: tauri::State<'_, SettingsStore>) -> Settings {
    store.get()
}

#[tauri::command]
pub fn set_theme(
    store: tauri::State<'_, SettingsStore>,
    theme: Theme,
) -> Result<Settings, String> {
    store
        .set_theme(theme)
        .map_err(|e| format!("could not save settings: {e}"))
}

#[tauri::command]
pub fn frontend_log(message: String) {
    eprintln!("calliope-ui: {}", sanitize_log(&message));
}

/// Replaces control characters with a space and truncates to 500 chars.
pub fn sanitize_log(msg: &str) -> String {
    msg.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(500)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_version_matches_build_version() {
        let v = app_version();
        assert_eq!(v, env!("CALLIOPE_VERSION"));
        let parts: Vec<&str> = v.split('.').collect();
        assert_eq!(parts.len(), 3);
        assert!(parts
            .iter()
            .all(|p| p.len() >= 2 && p.chars().all(|c| c.is_ascii_digit())));
        assert_eq!(parts[0].len(), 2);
        assert_eq!(parts[1].len(), 2);
        assert_eq!(parts[2].len(), 4);
    }

    #[test]
    fn sanitize_replaces_control_chars() {
        assert_eq!(sanitize_log("a\nb\rc\td\u{7}e\u{1b}f"), "a b c d e f");
        assert_eq!(sanitize_log("plain text"), "plain text");
    }

    #[test]
    fn sanitize_truncates_to_500_chars() {
        let long = "é".repeat(600);
        assert_eq!(sanitize_log(&long).chars().count(), 500);
    }
}
