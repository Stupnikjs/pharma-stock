//! Requêtes d'analyse sur le dernier snapshot (réutilisables par une future GUI).
use anyhow::Result;
use rusqlite::{params, Connection};

/// Identifiant du snapshot le plus récent (ordre d'import), s'il y en a un.
pub fn latest_snapshot(conn: &Connection) -> Result<Option<i64>> {
    Ok(conn.query_row("SELECT MAX(id) FROM snapshots", [], |r| r.get::<_, Option<i64>>(0))?)
}

/// Part du stock actif retrouvée dans la BDPM.
pub struct MatchRate {
    pub actives: i64,
    pub dans_bdpm: i64,
    pub dans_groupe: i64,
}

pub fn match_rate(conn: &Connection, snapshot_id: i64) -> Result<MatchRate> {
    Ok(conn.query_row(
        "SELECT COUNT(*),
                COALESCE(SUM(EXISTS(SELECT 1 FROM bdpm_cip c WHERE c.cip13 = l.cip13)), 0),
                COALESCE(SUM(EXISTS(SELECT 1 FROM bdpm_cip c
                                    JOIN bdpm_gener g ON g.cis = c.cis
                                    WHERE c.cip13 = l.cip13)), 0)
         FROM snapshot_lines l WHERE l.snapshot_id = ?1",
        [snapshot_id],
        |r| {
            Ok(MatchRate { actives: r.get(0)?, dans_bdpm: r.get(1)?, dans_groupe: r.get(2)? })
        },
    )?)
}

/// Agrégat par groupe générique (en BOÎTES, pas en unités).
pub struct GroupAgg {
    pub groupe_id: i64,
    pub libelle: String,
    pub nb_cip: i64,
    pub stock: i64,
    pub stock_princeps: i64,
    pub stock_generiques: i64,
    pub ventes: i64,
}

/// Top `limit` groupes par quantités vendues. Si un CIS appartenait à plusieurs
/// groupes, il serait compté dans chacun.
pub fn groups(conn: &Connection, snapshot_id: i64, limit: i64) -> Result<Vec<GroupAgg>> {
    let mut st = conn.prepare(
        "SELECT g.groupe_id, gg.libelle, COUNT(DISTINCT l.cip13),
                COALESCE(SUM(l.stock), 0),
                COALESCE(SUM(CASE WHEN g.type_gener = 0 THEN l.stock END), 0),
                COALESCE(SUM(CASE WHEN g.type_gener <> 0 THEN l.stock END), 0),
                COALESCE(SUM(l.qte_vendue), 0)
         FROM snapshot_lines l
         JOIN bdpm_cip c ON c.cip13 = l.cip13
         JOIN bdpm_gener g ON g.cis = c.cis
         JOIN bdpm_gener_group gg ON gg.groupe_id = g.groupe_id
         WHERE l.snapshot_id = ?1
         GROUP BY g.groupe_id
         ORDER BY 7 DESC, 4 DESC
         LIMIT ?2",
    )?;
    let rows = st.query_map(params![snapshot_id, limit], |r| {
        Ok(GroupAgg {
            groupe_id: r.get(0)?,
            libelle: r.get(1)?,
            nb_cip: r.get(2)?,
            stock: r.get(3)?,
            stock_princeps: r.get(4)?,
            stock_generiques: r.get(5)?,
            ventes: r.get(6)?,
        })
    })?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}

/// Ligne de la liste des snapshots importés.
pub struct SnapshotInfo {
    pub id: i64,
    pub date: String,
    pub seq: i64,
    pub lignes: i64,
    pub actives: i64,
    pub archive_file: String,
}

pub fn snapshots(conn: &Connection) -> Result<Vec<SnapshotInfo>> {
    let mut st = conn.prepare(
        "SELECT s.id, s.date_snapshot, s.seq, s.nb_lignes,
                (SELECT COUNT(*) FROM snapshot_lines l WHERE l.snapshot_id = s.id),
                s.archive_file
         FROM snapshots s ORDER BY s.id",
    )?;
    let rows = st.query_map([], |r| {
        Ok(SnapshotInfo {
            id: r.get(0)?,
            date: r.get(1)?,
            seq: r.get(2)?,
            lignes: r.get(3)?,
            actives: r.get(4)?,
            archive_file: r.get(5)?,
        })
    })?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}

/// Tailles du catalogue et du référentiel.
pub struct Counts {
    pub produits: i64,
    pub cis: i64,
    pub groupes: i64,
}

pub fn counts(conn: &Connection) -> Result<Counts> {
    let n = |sql: &str| -> Result<i64> { Ok(conn.query_row(sql, [], |r| r.get(0))?) };
    Ok(Counts {
        produits: n("SELECT COUNT(*) FROM products")?,
        cis: n("SELECT COUNT(*) FROM bdpm_cis")?,
        groupes: n("SELECT COUNT(*) FROM bdpm_gener_group")?,
    })
}
