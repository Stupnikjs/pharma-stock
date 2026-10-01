//! Diagnostic : référentiel chargé ? formats de CIP comparables ?
use crate::analysis;
use anyhow::Result;
use rusqlite::Connection;

type Rows<T> = std::result::Result<Vec<T>, rusqlite::Error>;

pub struct TableCount {
    pub table: &'static str,
    pub n: i64,
}
pub struct RefVersion {
    pub name: String,
    pub lignes: i64,
    pub rejets: i64,
    pub archive_file: String,
}
pub struct RejectCount {
    pub source: String,
    pub reason: String,
    pub n: i64,
}
pub struct PrefixStat {
    pub prefix: String,
    pub lignes: i64,
    pub valides: i64,
}
pub struct SampleLine {
    pub cip13: String,
    pub dans_bdpm: bool,
    pub libelle: String,
}

pub struct Diagnostic {
    pub tables: Vec<TableCount>,
    pub versions: Vec<RefVersion>,
    pub rejets: Vec<RejectCount>,
    pub snapshot_id: Option<i64>,
    pub prefixes: Vec<PrefixStat>,
    pub echantillon: Vec<SampleLine>,
    pub exemples_bdpm: Vec<(String, String)>,
}

pub fn diagnostic(conn: &Connection) -> Result<Diagnostic> {
    let mut tables = Vec::new();
    for t in ["bdpm_cis", "bdpm_gener_group", "bdpm_gener", "bdpm_cip"] {
        let n: i64 = conn.query_row(&format!("SELECT COUNT(*) FROM {t}"), [], |r| r.get(0))?;
        tables.push(TableCount { table: t, n });
    }

    let mut st = conn.prepare(
        "SELECT name, nb_lignes, nb_rejets, archive_file FROM ref_versions ORDER BY loaded_at",
    )?;
    let versions: Rows<RefVersion> = st
        .query_map([], |r| {
            Ok(RefVersion { name: r.get(0)?, lignes: r.get(1)?, rejets: r.get(2)?, archive_file: r.get(3)? })
        })?
        .collect();
    let versions = versions?;

    let mut st = conn.prepare(
        "SELECT source, reason, COUNT(*) FROM bdpm_rejects GROUP BY 1, 2 ORDER BY 3 DESC LIMIT 8",
    )?;
    let rejets: Rows<RejectCount> = st
        .query_map([], |r| Ok(RejectCount { source: r.get(0)?, reason: r.get(1)?, n: r.get(2)? }))?
        .collect();
    let rejets = rejets?;

    let mut st = conn.prepare("SELECT cip13, libelle_presentation FROM bdpm_cip LIMIT 3")?;
    let exemples: Rows<(String, String)> =
        st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect();
    let exemples_bdpm = exemples?;

    let snapshot_id = analysis::latest_snapshot(conn)?;
    let (mut prefixes, mut echantillon) = (Vec::new(), Vec::new());
    if let Some(sid) = snapshot_id {
        let mut st = conn.prepare(
            "SELECT substr(l.cip13, 1, 5), COUNT(*), COALESCE(SUM(p.cip_valide), 0)
             FROM snapshot_lines l JOIN products p ON p.cip13 = l.cip13
             WHERE l.snapshot_id = ?1 GROUP BY 1 ORDER BY 2 DESC LIMIT 8",
        )?;
        let rows: Rows<PrefixStat> = st
            .query_map([sid], |r| {
                Ok(PrefixStat { prefix: r.get(0)?, lignes: r.get(1)?, valides: r.get(2)? })
            })?
            .collect();
        prefixes = rows?;

        let mut st = conn.prepare(
            "SELECT l.cip13, EXISTS(SELECT 1 FROM bdpm_cip c WHERE c.cip13 = l.cip13), p.libelle
             FROM snapshot_lines l JOIN products p ON p.cip13 = l.cip13
             WHERE l.snapshot_id = ?1 ORDER BY l.qte_vendue DESC LIMIT 8",
        )?;
        let rows: Rows<SampleLine> = st
            .query_map([sid], |r| {
                Ok(SampleLine { cip13: r.get(0)?, dans_bdpm: r.get::<_, i64>(1)? == 1, libelle: r.get(2)? })
            })?
            .collect();
        echantillon = rows?;
    }

    Ok(Diagnostic { tables, versions, rejets, snapshot_id, prefixes, echantillon, exemples_bdpm })
}
