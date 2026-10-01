use crate::{archive, db, parse};
use anyhow::{Context, Result};
use rusqlite::Connection;
use std::path::{Path, PathBuf};

/// Lit tous les .txt de `raw_dir`, archive (zstd, nom horodaté) et importe ceux
/// qui ne sont pas déjà connus (détection par empreinte du contenu).
/// Les fichiers de `raw_dir` ne sont jamais modifiés ni supprimés.
pub fn sync(raw_dir: &Path, archive_dir: &Path, conn: &mut Connection) -> Result<()> {
    std::fs::create_dir_all(raw_dir)?;
    let mut files: Vec<PathBuf> = std::fs::read_dir(raw_dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_file() && p.extension().is_some_and(|x| x.eq_ignore_ascii_case("txt")))
        .collect();
    files.sort();

    if files.is_empty() {
        println!("{} : aucun .txt à traiter", raw_dir.display());
        return Ok(());
    }
    for f in files {
        let name = f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if let Err(e) = sync_one(&f, &name, archive_dir, conn) {
            eprintln!("{name} : ignoré ({e:#})");
        }
    }
    Ok(())
}

fn sync_one(path: &Path, name: &str, archive_dir: &Path, conn: &mut Connection) -> Result<()> {
    let bytes = std::fs::read(path).with_context(|| format!("lecture de {}", path.display()))?;
    let hash = archive::sha256_hex(&bytes);
    if db::has_hash(conn, &hash)? {
        println!("{name} : déjà archivé, ignoré");
        return Ok(());
    }

    let snap = parse::parse_snapshot(&parse::decode_cp850(&bytes))?;
    let seq = db::next_seq(conn, &snap.date)?;
    let archive_name = archive::archive_name(&snap.date, seq, &hash);
    let archive_path = archive::write(archive_dir, &archive_name, &bytes)?;

    match db::import(conn, &snap, name, &archive_name, &hash, seq) {
        Ok(st) => {
            println!("{name} -> {archive_name} (snapshot #{})", st.id);
            println!("  lignes          : {}", st.lignes);
            println!("  actives (stock/ventes <> 0) : {}", st.actives);
            println!("  sans CIP (archive seule)    : {}", st.sans_cip);
            Ok(())
        }
        Err(e) => {
            let _ = std::fs::remove_file(&archive_path); // pas d'archive orpheline
            Err(e)
        }
    }
}
