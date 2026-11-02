//! Desktop shell: hosts the web UI and one simulation session.
//!
//! The UI calls the `send` command with session commands and listens for
//! `pdsim` events. `pdsim-app <game>` loads a game at start.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use pdsim_session::{Command, Session};
use std::sync::Mutex;
use tauri::{Emitter, Manager};

struct AppSession(Mutex<Session>);

#[tauri::command]
fn send(state: tauri::State<'_, AppSession>, cmd: serde_json::Value) -> Result<(), String> {
    state.0.lock().map_err(|e| e.to_string())?.send_json(cmd)
}

fn main() {
    tauri::Builder::default()
        .setup(|app| {
            let handle = app.handle().clone();
            let session = Session::spawn(move |ev| {
                let _ = handle.emit("pdsim", ev);
            });
            if let Some(game) = std::env::args().nth(1) {
                session.send(Command::Load {
                    path: game,
                    calibration: None,
                });
            }
            app.manage(AppSession(Mutex::new(session)));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![send])
        .run(tauri::generate_context!())
        .expect("error while running the app");
}
