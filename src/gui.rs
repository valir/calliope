//! Tauri shell. The only module that touches Tauri/GTK.

pub fn run() {
    tauri::Builder::default()
        .run(tauri::generate_context!())
        .expect("error while running calliope-gui");
}
