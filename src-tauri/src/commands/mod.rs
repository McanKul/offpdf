//! Tauri command modules. Each `#[tauri::command]` here is registered in
//! `lib.rs`. Commands only ever pass file *paths* across IPC — never bytes.

pub mod files;
pub mod pdf;
pub mod jobs;
pub mod render;
