#![cfg(feature = "tauri")]

mod tauri_commands;

use tauri_commands::{
    join_package, join_session, list_windows, poll_package, poll_session, set_thought,
    start_package, start_session, stop_session, validate_ffmpeg, validate_session_name, GuiState,
};
use std::path::PathBuf;
use tauri::{WindowBuilder, WindowUrl};

fn start_static_server(dist_dir: PathBuf) {
    std::thread::spawn(move || {
        println!(
            "INFO static server: starting on http://127.0.0.1:4173 (dist_dir={})",
            dist_dir.display()
        );
        let server = match tiny_http::Server::http("127.0.0.1:4173") {
            Ok(server) => server,
            Err(err) => {
                eprintln!("ERROR static server: failed to bind: {err}");
                return;
            }
        };
        for request in server.incoming_requests() {
            let url = request.url();
            let path_part = url.split('?').next().unwrap_or(url);
            let rel = path_part.trim_start_matches('/');
            let rel = if rel.is_empty() { "index.html" } else { rel };
            let mut path = dist_dir.join(rel);
            if path.is_dir() {
                path = path.join("index.html");
            }
            let response = match std::fs::read(&path) {
                Ok(bytes) => {
                    let content_type = match path.extension().and_then(|ext| ext.to_str()) {
                        Some("html") => "text/html; charset=utf-8",
                        Some("css") => "text/css; charset=utf-8",
                        Some("js") => "application/javascript; charset=utf-8",
                        Some("ico") => "image/x-icon",
                        _ => "application/octet-stream",
                    };
                    let mut response = tiny_http::Response::from_data(bytes);
                    response.add_header(
                        tiny_http::Header::from_bytes("Content-Type", content_type).unwrap(),
                    );
                    response
                }
                Err(_) => {
                    eprintln!(
                        "WARN static server: 404 (url={}, path={})",
                        url,
                        path.display()
                    );
                    tiny_http::Response::from_string("Not Found").with_status_code(404)
                }
            };
            let _ = request.respond(response);
        }
    });
}
fn main() {
    println!("INFO gui: starting");
    tauri::Builder::default()
        .manage(GuiState::default())
        .setup(|app| {
            println!("INFO gui: setup");
            start_static_server(PathBuf::from("./gui/dist"));
            WindowBuilder::new(
                app,
                "main",
                WindowUrl::External("http://127.0.0.1:4173/".parse().unwrap()),
            )
                .title("Collector GUI")
                .inner_size(1200.0, 760.0)
                .resizable(true)
                .build()?;
            println!("INFO gui: main window created");
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            start_session,
            poll_session,
            join_session,
            stop_session,
            set_thought,
            validate_ffmpeg,
            validate_session_name,
            start_package,
            poll_package,
            join_package,
            list_windows
        ])
        .run(tauri::generate_context!())
        .expect("tauri app failed");
}
