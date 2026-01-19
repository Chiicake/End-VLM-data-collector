#![cfg(feature = "tauri")]

mod tauri_commands;

use tauri_commands::{
    join_package, join_session, list_windows, poll_package, poll_session, set_thought,
    start_package, start_session, GuiState,
};

fn main() {
    tauri::Builder::default()
        .manage(GuiState::default())
        .invoke_handler(tauri::generate_handler![
            start_session,
            poll_session,
            join_session,
            set_thought,
            start_package,
            poll_package,
            join_package,
            list_windows
        ])
        .run(tauri::generate_context!())
        .expect("tauri app failed");
}
