use anyhow::{Context, Result};
use pharma_stock::{bdpm, db, sync};
use rusqlite::Connection;
use std::path::{Path, PathBuf};

const USAGE: &str = "Usage :
  pharma_stock [--db data/stock.db] [--raw data/raw] [--archive data/archive] [--ref data/ref] [stats]

À chaque run : les .txt de data/raw sont archivés (zstd) et importés s'ils sont nouveaux ;
les fichiers BDPM de data/ref (CIS_bdpm.txt, CIS_GENER_bdpm.txt) sont chargés s'ils ont changé.";

fn main() -> Result<()> {
    // Régénère forllm.txt à chaque run (non bloquant en cas d'erreur)
    if let Err(e) = dump_sources() {
        eprintln!("forllm.txt : {e:#}");
    }

    let mut db_path = "data/stock.db".to_string();
    let mut raw = "data/raw".to_string();
    let mut archive = "data/archive".to_string();
    let mut refdir = "data/ref".to_string();
    let mut rest: Vec<String> = Vec::new();
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--db" => db_path = it.next().context("--db attend un chemin")?,
            "--raw" => raw = it.next().context("--raw attend un chemin")?,
            "--archive" => archive = it.next().context("--archive attend un chemin")?,
            "--ref" => refdir = it.next().context("--ref attend un chemin")?,
            _ => rest.push(a),
        }
    }

    let mut conn = db::open(Path::new(&db_path))?;
    bdpm::sync_ref(Path::new(&refdir), Path::new(&archive), &mut conn)?;
    sync::sync(Path::new(&raw), Path::new(&archive), &mut conn)?;

    match rest.first().map(String::as_str) {
        None => Ok(()),
        Some("stats") => cmd_stats(&conn),
        Some(other) => {
            eprintln!("commande inconnue : {other}\n{USAGE}");
            Ok(())
        }
    }
}

/// Liste récursivement les fichiers .rs d'un dossier.
fn collect_rs(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let p = entry?.path();
        if p.is_dir() {
            collect_rs(&p, out)?;
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
    Ok(())
}

/// Concatène tous les .rs de src/ dans forllm.txt (à la racine du projet).
fn dump_sources() -> Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    collect_rs(&root.join("src"), &mut files)?;
    files.sort();

    let mut out = String::new();
    for f in &files {
        let rel = f.strip_prefix(root).unwrap_or(f);
        out.push_str(&format!("// ===== {} =====\n", rel.display()));
        out.push_str(&std::fs::read_to_string(f)?);
        if !out.ends_with('\n') {
            out.push('\n');
        }
        out.push('\n');
    }
    std::fs::write(root.join("forllm.txt"), out)?;
    Ok(())
}

fn cmd_stats(conn: &Connection) -> Result<()> {
    let mut st = conn.prepare(
        "SELECT s.id, s.date_snapshot, s.seq, s.nb_lignes,
                (SELECT COUNT(*) FROM snapshot_lines l WHERE l.snapshot_id = s.id),
                s.archive_file
         FROM snapshots s ORDER BY s.id",
    )?;
    let rows = st.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, i64>(2)?,
            r.get::<_, i64>(3)?,
            r.get::<_, i64>(4)?,
            r.get::<_, String>(5)?,
        ))
    })?;
    println!("{:>4}  {:10}  {:>3}  {:>8}  {:>8}  archive", "id", "date", "n°", "lignes", "actives");
    for row in rows {
        let (id, date, seq, n, act, file) = row?;
        println!("{id:>4}  {date:10}  {seq:>3}  {n:>8}  {act:>8}  {file}");
    }
    let produits: i64 = conn.query_row("SELECT COUNT(*) FROM products", [], |r| r.get(0))?;
    println!("Catalogue : {produits} produits");
    let nb_cis: i64 = conn.query_row("SELECT COUNT(*) FROM bdpm_cis", [], |r| r.get(0))?;
    let nb_gr: i64 = conn.query_row("SELECT COUNT(*) FROM bdpm_gener_group", [], |r| r.get(0))?;
    println!("Référentiel BDPM : {nb_cis} CIS, {nb_gr} groupes génériques");
    Ok(())
}
