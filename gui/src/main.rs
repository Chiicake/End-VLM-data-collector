#![cfg(feature = "tauri")]

use gui::tauri_commands::{
    join_package, join_session, poll_package, poll_session, start_package, start_session, GuiState,
};

fn main() {
    tauri::Builder::default()
        .manage(GuiState::default())
        .invoke_handler(tauri::generate_handler![
            start_session,
            poll_session,
            join_session,
            start_package,
            poll_package,
            join_package
        ])
        .run(tauri::generate_context!())
        .expect("tauri app failed");
}
