mod devtools;

use anyhow::{Context, Result};
use pharma_core::bdpm::{self, RefOutcome, RefReport};
use pharma_core::sync::{self, FileOutcome, SyncReport};
use pharma_core::{analysis, db, diag, Config, Connection};

const USAGE: &str = "Usage :
  pharma_stock [--db data/stock.db] [--raw data/raw] [--archive data/archive] [--ref data/ref]
               [stats | rapprochement | groupes [N] | diag]

À chaque run : les fichiers BDPM de data/ref (CIS_bdpm, CIS_GENER_bdpm, CIS_CIP_bdpm)
sont chargés s'ils ont changé, puis les .txt de data/raw sont archivés (zstd) et importés
s'ils sont nouveaux.";

fn main() -> Result<()> {
    // Régénère forllm.txt à chaque run (non bloquant en cas d'erreur)
    if let Err(e) = devtools::dump_sources() {
        eprintln!("forllm.txt : {e:#}");
    }

    let (cfg, rest) = parse_args()?;
    let mut conn = db::open(&cfg.db)?;

    print_ref_report(&bdpm::sync_ref(&cfg.refdir, &cfg.archive, &mut conn)?);
    print_sync_report(&sync::sync(&cfg.raw, &cfg.archive, &mut conn)?, &cfg);

    match rest.first().map(String::as_str) {
        None => Ok(()),
        Some("stats") => cmd_stats(&conn),
        Some("rapprochement") => cmd_rapprochement(&conn),
        Some("groupes") => {
            let n = rest.get(1).and_then(|x| x.parse().ok()).unwrap_or(20);
            cmd_groupes(&conn, n)
        }
        Some("diag") => cmd_diag(&conn),
        Some(other) => {
            eprintln!("commande inconnue : {other}\n{USAGE}");
            Ok(())
        }
    }
}

fn parse_args() -> Result<(Config, Vec<String>)> {
    let mut cfg = Config::default();
    let mut rest = Vec::new();
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--db" => cfg.db = it.next().context("--db attend un chemin")?.into(),
            "--raw" => cfg.raw = it.next().context("--raw attend un chemin")?.into(),
            "--archive" => cfg.archive = it.next().context("--archive attend un chemin")?.into(),
            "--ref" => cfg.refdir = it.next().context("--ref attend un chemin")?.into(),
            _ => rest.push(a),
        }
    }
    Ok((cfg, rest))
}

// ------------------------------------------------------------------ rapports

fn print_ref_report(r: &RefReport) {
    for f in &r.files {
        match &f.outcome {
            RefOutcome::Loaded { archive_file, lignes, rejets, .. } => {
                println!("{} -> {archive_file} : {lignes} lignes, {rejets} rejetées", f.file)
            }
            RefOutcome::AlreadyKnown { .. } => println!("{} : déjà chargé, ignoré", f.file),
            RefOutcome::Failed(e) => eprintln!("{} : ignoré ({e})", f.file),
            RefOutcome::Unrecognized => eprintln!(
                "{} : fichier non reconnu (attendus : CIS_bdpm, CIS_GENER_bdpm, CIS_CIP_bdpm)",
                f.file
            ),
        }
    }
}

fn print_sync_report(r: &SyncReport, cfg: &Config) {
    if r.files.is_empty() {
        println!("{} : aucun .txt à traiter", cfg.raw.display());
    }
    for f in &r.files {
        match &f.outcome {
            FileOutcome::Imported { snapshot_id, date, archive_file, lignes, actives, sans_cip } => {
                println!("{} -> {archive_file} (snapshot #{snapshot_id}, {date})", f.file);
                println!("  lignes          : {lignes}");
                println!("  actives (stock/ventes <> 0) : {actives}");
                println!("  sans CIP (archive seule)    : {sans_cip}");
            }
            FileOutcome::AlreadyKnown => println!("{} : déjà archivé, ignoré", f.file),
            FileOutcome::Failed(e) => eprintln!("{} : ignoré ({e})", f.file),
        }
    }
}

// ----------------------------------------------------------------- commandes

fn pct(a: i64, b: i64) -> f64 {
    if b == 0 {
        0.0
    } else {
        100.0 * a as f64 / b as f64
    }
}

fn cmd_stats(conn: &Connection) -> Result<()> {
    println!("{:>4}  {:10}  {:>3}  {:>8}  {:>8}  archive", "id", "date", "n°", "lignes", "actives");
    for s in analysis::snapshots(conn)? {
        println!(
            "{:>4}  {:10}  {:>3}  {:>8}  {:>8}  {}",
            s.id, s.date, s.seq, s.lignes, s.actives, s.archive_file
        );
    }
    let c = analysis::counts(conn)?;
    println!("Catalogue : {} produits", c.produits);
    println!("Référentiel BDPM : {} CIS, {} groupes génériques", c.cis, c.groupes);
    Ok(())
}

fn cmd_rapprochement(conn: &Connection) -> Result<()> {
    let Some(sid) = analysis::latest_snapshot(conn)? else {
        println!("Aucun snapshot importé.");
        return Ok(());
    };
    let m = analysis::match_rate(conn, sid)?;
    println!("Snapshot #{sid} : {} lignes actives (stock ou ventes <> 0)", m.actives);
    println!("  retrouvées dans la BDPM (CIS_CIP) : {} ({:.1} %)", m.dans_bdpm, pct(m.dans_bdpm, m.actives));
    println!("  appartenant à un groupe générique : {} ({:.1} %)", m.dans_groupe, pct(m.dans_groupe, m.actives));
    Ok(())
}

fn cmd_groupes(conn: &Connection, n: i64) -> Result<()> {
    let Some(sid) = analysis::latest_snapshot(conn)? else {
        println!("Aucun snapshot importé.");
        return Ok(());
    };
    println!("Snapshot #{sid} : top {n} groupes par quantités vendues (en boîtes)");
    println!(
        "{:>6} {:>4} {:>7} {:>8} {:>8} {:>7}  libellé",
        "groupe", "cip", "stock", "princeps", "génériq.", "ventes"
    );
    for g in analysis::groups(conn, sid, n)? {
        let lib: String = g.libelle.chars().take(60).collect();
        println!(
            "{:>6} {:>4} {:>7} {:>8} {:>8} {:>7}  {}",
            g.groupe_id, g.nb_cip, g.stock, g.stock_princeps, g.stock_generiques, g.ventes, lib
        );
    }
    Ok(())
}

fn cmd_diag(conn: &Connection) -> Result<()> {
    let d = diag::diagnostic(conn)?;
    println!("== Référentiel BDPM ==");
    for t in &d.tables {
        println!("  {:<18} {}", t.table, t.n);
    }
    println!("  versions chargées :");
    if d.versions.is_empty() {
        println!("    aucune : data/ref est vide, ou introuvable depuis le dossier courant");
    }
    for v in &d.versions {
        println!("    {} : {} lignes, {} rejets -> {}", v.name, v.lignes, v.rejets, v.archive_file);
    }
    for r in &d.rejets {
        println!("  rejet {} : {} ({})", r.source, r.reason, r.n);
    }

    println!("== Stock (dernier snapshot) ==");
    if d.snapshot_id.is_none() {
        println!("  aucun snapshot");
        return Ok(());
    }
    println!("  préfixes CIP13 des lignes actives (préfixe, lignes, dont CIP valides) :");
    for p in &d.prefixes {
        println!("    {}  {:>6}  {:>6}", p.prefix, p.lignes, p.valides);
    }
    println!("  8 premières lignes actives (CIP13, dans BDPM ?, libellé) :");
    for l in &d.echantillon {
        println!("    {}  {}  {}", l.cip13, if l.dans_bdpm { "oui" } else { "non" }, l.libelle);
    }
    println!("== BDPM : 3 exemples de CIP13 ==");
    for (cip, lib) in &d.exemples_bdpm {
        println!("    {cip}  {lib}");
    }
    Ok(())
}
