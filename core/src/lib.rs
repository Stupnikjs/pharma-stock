//! Cœur du projet : parsing des exports SmartRx, archivage, référentiel BDPM,
//! stockage SQLite et analyses. Aucune sortie console : tout est renvoyé sous
//! forme de structures, pour que la CLI comme une future GUI puissent les afficher.
pub mod analysis;
pub mod archive;
pub mod bdpm;
pub mod config;
pub mod db;
pub mod diag;
pub mod parse;
pub mod sync;

pub use config::Config;
pub use rusqlite::Connection;
