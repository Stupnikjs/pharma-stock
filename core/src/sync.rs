use crate::{archive, db, parse};
use anyhow::{Context, Result};
use rusqlite::Connection;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub enum FileOutcome {
    Imported {
        snapshot_id: i64,
        date: String,
        archive_file: String,
        lignes: usize,
        actives: usize,
        sans_cip: usize,
    },
    /// Même contenu (empreinte) déjà importé.
    AlreadyKnown,
    Failed(String),
}

#[derive(Debug)]
pub struct FileReport {
    pub file: String,
    pub outcome: FileOutcome,
}

#[derive(Debug, Default)]
pub struct SyncReport {
    pub files: Vec<FileReport>,
}

impl SyncReport {
    pub fn imported(&self) -> usize {
        self.files.iter().filter(|f| matches!(f.outcome, FileOutcome::Imported { .. })).count()
    }
    pub fn failed(&self) -> usize {
        self.files.iter().filter(|f| matches!(f.outcome, FileOutcome::Failed(_))).count()
    }
}

/// Avancement, pour une barre de progression côté GUI.
pub struct Progress<'a> {
    pub index: usize,
    pub total: usize,
    pub file: &'a str,
}

pub fn sync(raw_dir: &Path, archive_dir: &Path, conn: &mut Connection) -> Result<SyncReport> {
    sync_with(raw_dir, archive_dir, conn, &mut |_| {})
}

/// Lit tous les .txt de `raw_dir`, archive (zstd, nom horodaté) et importe ceux
/// qui ne sont pas déjà connus (détection par empreinte du contenu).
/// Les fichiers de `raw_dir` ne sont jamais modifiés ni supprimés.
pub fn sync_with(
    raw_dir: &Path,
    archive_dir: &Path,
    conn: &mut Connection,
    progress: &mut dyn FnMut(&Progress),
) -> Result<SyncReport> {
    std::fs::create_dir_all(raw_dir)?;
    let mut files: Vec<PathBuf> = std::fs::read_dir(raw_dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_file() && p.extension().is_some_and(|x| x.eq_ignore_ascii_case("txt")))
        .collect();
    files.sort();

    let total = files.len();
    let mut report = SyncReport::default();
    for (i, f) in files.iter().enumerate() {
        let name = f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        progress(&Progress { index: i + 1, total, file: &name });
        let outcome = match sync_one(f, &name, archive_dir, conn) {
            Ok(o) => o,
            Err(e) => FileOutcome::Failed(format!("{e:#}")),
        };
        report.files.push(FileReport { file: name, outcome });
    }
    Ok(report)
}

fn sync_one(path: &Path, name: &str, archive_dir: &Path, conn: &mut Connection) -> Result<FileOutcome> {
    let bytes = std::fs::read(path).with_context(|| format!("lecture de {}", path.display()))?;
    let hash = archive::sha256_hex(&bytes);
    if db::has_hash(conn, &hash)? {
        return Ok(FileOutcome::AlreadyKnown);
    }

    let snap = parse::parse_snapshot(&parse::decode_cp850(&bytes))?;
    let seq = db::next_seq(conn, &snap.date)?;
    let archive_name = archive::archive_name(&snap.date, seq, &hash);
    let archive_path = archive::write(archive_dir, &archive_name, &bytes)?;

    match db::import(conn, &snap, name, &archive_name, &hash, seq) {
        Ok(st) => Ok(FileOutcome::Imported {
            snapshot_id: st.id,
            date: snap.date,
            archive_file: archive_name,
            lignes: st.lignes,
            actives: st.actives,
            sans_cip: st.sans_cip,
        }),
        Err(e) => {
            let _ = std::fs::remove_file(&archive_path); // pas d'archive orpheline
            Err(e)
        }
    }
}
