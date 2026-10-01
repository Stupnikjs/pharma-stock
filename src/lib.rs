//! Cœur du projet : parsing des exports SmartRx, archivage et stockage SQLite.
//! Pensé pour être réutilisé tel quel par une future GUI (egui, axum, Tauri...).
pub mod archive;
pub mod bdpm;
pub mod db;
pub mod parse;
pub mod sync;
