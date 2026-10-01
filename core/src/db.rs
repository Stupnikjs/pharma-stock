use crate::parse::Snapshot;
use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS snapshots(
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    date_snapshot TEXT NOT NULL,
    seq           INTEGER NOT NULL,
    content_hash  TEXT NOT NULL UNIQUE,
    source_file   TEXT NOT NULL,
    archive_file  TEXT NOT NULL,
    nb_lignes     INTEGER NOT NULL,
    imported_at   TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(date_snapshot, seq)
);
-- Catalogue : une seule ligne par produit (dernières valeurs connues)
CREATE TABLE IF NOT EXISTS products(
    cip13            TEXT PRIMARY KEY,
    libelle          TEXT NOT NULL,
    lis              TEXT,
    cdt              INTEGER,
    rbt              TEXT,
    tva              REAL,
    px_rem_ug_c      INTEGER,
    px_moy_pond_c    INTEGER,
    pu_vente_c       INTEGER,
    cip_valide       INTEGER NOT NULL,
    code_interne     INTEGER NOT NULL,
    libelle_est_code INTEGER NOT NULL,
    first_seen       INTEGER NOT NULL REFERENCES snapshots(id),
    last_seen        INTEGER NOT NULL REFERENCES snapshots(id)
);
-- Lignes clairsemées : absent = stock 0 et ventes 0. Prix en centimes.
CREATE TABLE IF NOT EXISTS snapshot_lines(
    snapshot_id   INTEGER NOT NULL REFERENCES snapshots(id),
    cip13         TEXT NOT NULL REFERENCES products(cip13),
    stock         INTEGER,
    qte_vendue    INTEGER,
    px_moy_pond_c INTEGER,
    pu_vente_c    INTEGER,
    PRIMARY KEY(snapshot_id, cip13)
) WITHOUT ROWID;

-- Référentiel BDPM (remplacé en bloc à chaque nouvelle version du fichier)
CREATE TABLE IF NOT EXISTS ref_versions(
    name         TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    archive_file TEXT NOT NULL,
    nb_lignes    INTEGER NOT NULL,
    nb_rejets    INTEGER NOT NULL,
    loaded_at    TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY(name, content_hash)
);
CREATE TABLE IF NOT EXISTS bdpm_cis(
    cis                    TEXT PRIMARY KEY,
    denomination           TEXT NOT NULL,
    forme                  TEXT,
    voies                  TEXT,
    statut_amm             TEXT,
    procedure_amm          TEXT,
    etat_commercialisation TEXT,
    date_amm               TEXT,
    statut_bdm             TEXT,
    titulaire              TEXT,
    surveillance_renforcee INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS bdpm_gener_group(
    groupe_id INTEGER PRIMARY KEY,
    libelle   TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS bdpm_gener(
    groupe_id  INTEGER NOT NULL REFERENCES bdpm_gener_group(groupe_id),
    cis        TEXT NOT NULL,
    type_gener INTEGER NOT NULL,
    tri        INTEGER NOT NULL,
    PRIMARY KEY(groupe_id, cis)
) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS idx_gener_cis ON bdpm_gener(cis);
CREATE TABLE IF NOT EXISTS bdpm_cip(
    cip13                  TEXT PRIMARY KEY,
    cip7                   TEXT NOT NULL,
    cis                    TEXT NOT NULL,
    libelle_presentation   TEXT NOT NULL,
    statut_presentation    TEXT NOT NULL,
    etat_commercialisation TEXT NOT NULL,
    date_declaration       TEXT,
    agrement_collectivites INTEGER NOT NULL,
    taux_remb              INTEGER,
    prix_c                 INTEGER,
    prix_hono_c            INTEGER,
    honoraires_c           INTEGER,
    indications_remb       TEXT
);
CREATE INDEX IF NOT EXISTS idx_cip_cis ON bdpm_cip(cis);
CREATE TABLE IF NOT EXISTS bdpm_rejects(
    source TEXT NOT NULL,
    line   INTEGER NOT NULL,
    raw    TEXT NOT NULL,
    reason TEXT NOT NULL
);
";

pub fn open(path: &Path) -> Result<Connection> {
    if let Some(dir) = path.parent() {
        if !dir.as_os_str().is_empty() {
            std::fs::create_dir_all(dir)?;
        }
    }
    let conn = Connection::open(path).with_context(|| format!("ouverture de {}", path.display()))?;
    conn.execute_batch(SCHEMA)?;
    Ok(conn)
}

pub fn has_hash(conn: &Connection, hash: &str) -> Result<bool> {
    let found: Option<i64> = conn
        .query_row("SELECT id FROM snapshots WHERE content_hash = ?1", [hash], |r| r.get(0))
        .optional()?;
    Ok(found.is_some())
}

/// Prochain numéro d'ordre pour une date donnée (1, 2, 3...).
pub fn next_seq(conn: &Connection, date: &str) -> Result<i64> {
    Ok(conn.query_row(
        "SELECT COALESCE(MAX(seq), 0) + 1 FROM snapshots WHERE date_snapshot = ?1",
        [date],
        |r| r.get(0),
    )?)
}

pub struct ImportStats {
    pub id: i64,
    pub lignes: usize,
    pub sans_cip: usize,
    pub actives: usize,
}

fn cents(x: Option<f64>) -> Option<i64> {
    x.map(|v| (v * 100.0).round() as i64)
}

/// Importe un snapshot en une transaction (catalogue + lignes actives).
pub fn import(
    conn: &mut Connection,
    snap: &Snapshot,
    source_file: &str,
    archive_file: &str,
    hash: &str,
    seq: i64,
) -> Result<ImportStats> {
    let tx = conn.transaction()?;
    tx.execute(
        "INSERT INTO snapshots(date_snapshot, seq, content_hash, source_file, archive_file, nb_lignes)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![snap.date, seq, hash, source_file, archive_file, snap.products.len() as i64],
    )
    .context("insertion du snapshot")?;
    let id = tx.last_insert_rowid();

    let mut sans_cip = 0usize;
    let mut actives = 0usize;
    {
        let mut up = tx.prepare(
            "INSERT INTO products(cip13, libelle, lis, cdt, rbt, tva, px_rem_ug_c, px_moy_pond_c,
                pu_vente_c, cip_valide, code_interne, libelle_est_code, first_seen, last_seen)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?13)
             ON CONFLICT(cip13) DO UPDATE SET
                libelle          = CASE WHEN excluded.libelle <> '' THEN excluded.libelle ELSE products.libelle END,
                lis              = COALESCE(excluded.lis, products.lis),
                cdt              = COALESCE(excluded.cdt, products.cdt),
                rbt              = COALESCE(excluded.rbt, products.rbt),
                tva              = COALESCE(excluded.tva, products.tva),
                px_rem_ug_c      = COALESCE(excluded.px_rem_ug_c, products.px_rem_ug_c),
                px_moy_pond_c    = COALESCE(excluded.px_moy_pond_c, products.px_moy_pond_c),
                pu_vente_c       = COALESCE(excluded.pu_vente_c, products.pu_vente_c),
                libelle_est_code = excluded.libelle_est_code,
                last_seen        = excluded.last_seen",
        )?;
        let mut ln = tx.prepare(
            "INSERT OR REPLACE INTO snapshot_lines(snapshot_id, cip13, stock, qte_vendue,
                px_moy_pond_c, pu_vente_c)
             VALUES (?1,?2,?3,?4,?5,?6)",
        )?;

        for p in &snap.products {
            if p.cip13.is_empty() {
                sans_cip += 1; // lignes sans CIP : conservées uniquement dans l'archive brute
                continue;
            }
            up.execute(params![
                p.cip13,
                p.libelle,
                p.lis,
                p.cdt,
                p.rbt,
                p.tva,
                cents(p.px_rem_ug),
                cents(p.px_moy_pond),
                cents(p.pu_vente),
                p.cip_valide,
                p.code_interne,
                p.libelle_est_code,
                id,
            ])?;
            if p.stock.unwrap_or(0) != 0 || p.qte_vendue.unwrap_or(0) != 0 {
                ln.execute(params![
                    id,
                    p.cip13,
                    p.stock,
                    p.qte_vendue,
                    cents(p.px_moy_pond),
                    cents(p.pu_vente),
                ])?;
                actives += 1;
            }
        }
    }
    tx.commit()?;
    Ok(ImportStats { id, lignes: snap.products.len(), sans_cip, actives })
}
