//! Référentiel BDPM : chargement de CIS_bdpm.txt et CIS_GENER_bdpm.txt.
//! (CIS_CIP_bdpm.txt, qui relie CIP13 et CIS, viendra ensuite.)
use crate::archive;
use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use std::path::Path;

pub const FILE_CIS: &str = "CIS_bdpm.txt";
pub const FILE_GENER: &str = "CIS_GENER_bdpm.txt";

// ---------------------------------------------------------------- décodage

/// Moitié 0x80..=0x9F de Windows-1252 (le reste de 0xA0..=0xFF = Latin-1).
const CP1252_C1: [char; 32] = [
    '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8D}', 'Ž', '\u{8F}',
    '\u{90}', '‘', '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\u{9D}', 'ž', 'Ÿ',
];

/// UTF-8 si le contenu l'est, sinon Windows-1252 / Latin-1 (cas habituel de la BDPM).
pub fn decode_text(bytes: &[u8]) -> String {
    if let Ok(s) = std::str::from_utf8(bytes) {
        return s.trim_start_matches('\u{feff}').to_string();
    }
    bytes
        .iter()
        .map(|&b| match b {
            0x80..=0x9F => CP1252_C1[(b - 0x80) as usize],
            _ => b as char,
        })
        .collect()
}

// ------------------------------------------------------------------ types

#[derive(Debug, Clone, PartialEq)]
pub struct CisRow {
    pub cis: String,
    pub denomination: String,
    pub forme: String,
    pub voies: String,
    pub statut_amm: String,
    pub procedure_amm: String,
    pub etat_commercialisation: String,
    pub date_amm: Option<String>,
    pub statut_bdm: Option<String>,
    pub titulaire: String,
    pub surveillance_renforcee: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GenerRow {
    pub groupe_id: i64,
    pub libelle: String,
    pub cis: String,
    /// 0 = princeps, 1 = générique ; les autres valeurs sont à vérifier dans la notice BDPM.
    pub type_gener: i64,
    pub tri: i64,
}

#[derive(Debug)]
pub struct Rejected {
    pub line: usize,
    pub raw: String,
    pub reason: String,
}

#[derive(Debug)]
pub struct Parsed<T> {
    pub rows: Vec<T>,
    pub rejects: Vec<Rejected>,
}

// ---------------------------------------------------------------- parsing

fn rej(line: usize, raw: &str, reason: String) -> Rejected {
    Rejected { line, raw: raw.chars().take(300).collect(), reason }
}

fn opt(s: &str) -> Option<String> {
    let t = s.trim();
    if t.is_empty() {
        None
    } else {
        Some(t.to_string())
    }
}

fn is_cis(s: &str) -> bool {
    s.len() == 8 && s.chars().all(|c| c.is_ascii_digit())
}

/// "12/03/1998" -> "1998-03-12"
fn date_fr(s: &str) -> Option<String> {
    let mut it = s.trim().split('/');
    let d: u32 = it.next()?.parse().ok()?;
    let m: u32 = it.next()?.parse().ok()?;
    let y: u32 = it.next()?.parse().ok()?;
    if it.next().is_some() || !(1..=31).contains(&d) || !(1..=12).contains(&m) {
        return None;
    }
    Some(format!("{y:04}-{m:02}-{d:02}"))
}

pub fn parse_cis(text: &str) -> Parsed<CisRow> {
    let (mut rows, mut rejects) = (Vec::new(), Vec::new());
    for (i, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() < 12 {
            rejects.push(rej(i + 1, line, format!("{} colonnes au lieu de 12", f.len())));
            continue;
        }
        let cis = f[0].trim();
        if !is_cis(cis) {
            rejects.push(rej(i + 1, line, "code CIS invalide".into()));
            continue;
        }
        rows.push(CisRow {
            cis: cis.to_string(),
            denomination: f[1].trim().to_string(),
            forme: f[2].trim().to_string(),
            voies: f[3].trim().to_string(),
            statut_amm: f[4].trim().to_string(),
            procedure_amm: f[5].trim().to_string(),
            etat_commercialisation: f[6].trim().to_string(),
            date_amm: date_fr(f[7]),
            statut_bdm: opt(f[8]),
            titulaire: f[10].trim().to_string(),
            surveillance_renforcee: f[11].trim().eq_ignore_ascii_case("oui"),
        });
    }
    Parsed { rows, rejects }
}

pub fn parse_gener(text: &str) -> Parsed<GenerRow> {
    let (mut rows, mut rejects) = (Vec::new(), Vec::new());
    for (i, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() < 5 {
            rejects.push(rej(i + 1, line, format!("{} colonnes au lieu de 5", f.len())));
            continue;
        }
        let (groupe, type_g, tri) = (
            f[0].trim().parse::<i64>(),
            f[3].trim().parse::<i64>(),
            f[4].trim().parse::<i64>(),
        );
        let cis = f[2].trim();
        match (groupe, type_g, tri) {
            (Ok(groupe_id), Ok(type_gener), Ok(tri)) if is_cis(cis) => rows.push(GenerRow {
                groupe_id,
                libelle: f[1].trim().to_string(),
                cis: cis.to_string(),
                type_gener,
                tri,
            }),
            _ => rejects.push(rej(i + 1, line, "champ numérique ou CIS invalide".into())),
        }
    }
    Parsed { rows, rejects }
}

// ------------------------------------------------------------- chargement

fn finish(
    tx: &Transaction,
    name: &str,
    hash: &str,
    archive_file: &str,
    nb: usize,
    rejects: &[Rejected],
) -> Result<()> {
    tx.execute("DELETE FROM bdpm_rejects WHERE source = ?1", [name])?;
    for r in rejects.iter().take(1000) {
        tx.execute(
            "INSERT INTO bdpm_rejects(source, line, raw, reason) VALUES (?1,?2,?3,?4)",
            params![name, r.line as i64, r.raw, r.reason],
        )?;
    }
    tx.execute(
        "INSERT INTO ref_versions(name, content_hash, archive_file, nb_lignes, nb_rejets)
         VALUES (?1,?2,?3,?4,?5)",
        params![name, hash, archive_file, nb as i64, rejects.len() as i64],
    )?;
    Ok(())
}

pub fn load_cis(conn: &mut Connection, p: &Parsed<CisRow>, hash: &str, archive_file: &str) -> Result<()> {
    let tx = conn.transaction()?;
    tx.execute("DELETE FROM bdpm_cis", [])?;
    {
        let mut st = tx.prepare(
            "INSERT OR REPLACE INTO bdpm_cis(cis, denomination, forme, voies, statut_amm,
                procedure_amm, etat_commercialisation, date_amm, statut_bdm, titulaire,
                surveillance_renforcee)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
        )?;
        for r in &p.rows {
            st.execute(params![
                r.cis,
                r.denomination,
                r.forme,
                r.voies,
                r.statut_amm,
                r.procedure_amm,
                r.etat_commercialisation,
                r.date_amm,
                r.statut_bdm,
                r.titulaire,
                r.surveillance_renforcee,
            ])?;
        }
    }
    finish(&tx, FILE_CIS, hash, archive_file, p.rows.len(), &p.rejects)?;
    tx.commit()?;
    Ok(())
}

pub fn load_gener(conn: &mut Connection, p: &Parsed<GenerRow>, hash: &str, archive_file: &str) -> Result<()> {
    let tx = conn.transaction()?;
    tx.execute("DELETE FROM bdpm_gener", [])?;
    tx.execute("DELETE FROM bdpm_gener_group", [])?;
    {
        let mut g = tx.prepare("INSERT OR REPLACE INTO bdpm_gener_group(groupe_id, libelle) VALUES (?1,?2)")?;
        let mut l = tx.prepare(
            "INSERT OR REPLACE INTO bdpm_gener(groupe_id, cis, type_gener, tri) VALUES (?1,?2,?3,?4)",
        )?;
        for r in &p.rows {
            g.execute(params![r.groupe_id, r.libelle])?;
            l.execute(params![r.groupe_id, r.cis, r.type_gener, r.tri])?;
        }
    }
    finish(&tx, FILE_GENER, hash, archive_file, p.rows.len(), &p.rejects)?;
    tx.commit()?;
    Ok(())
}

/// Charge les fichiers BDPM présents dans `ref_dir` (s'il existe) : archive zstd
/// dans `<archive_dir>/ref/`, puis remplacement complet de la table concernée.
/// Un fichier déjà chargé (même empreinte) est ignoré.
pub fn sync_ref(ref_dir: &Path, archive_dir: &Path, conn: &mut Connection) -> Result<()> {
    if !ref_dir.is_dir() {
        return Ok(());
    }
    for name in [FILE_CIS, FILE_GENER] {
        let path = ref_dir.join(name);
        if path.is_file() {
            if let Err(e) = sync_ref_one(&path, name, archive_dir, conn) {
                eprintln!("{name} : ignoré ({e:#})");
            }
        }
    }
    Ok(())
}

fn sync_ref_one(path: &Path, name: &str, archive_dir: &Path, conn: &mut Connection) -> Result<()> {
    let bytes = std::fs::read(path).with_context(|| format!("lecture de {}", path.display()))?;
    let hash = archive::sha256_hex(&bytes);
    let known: Option<i64> = conn
        .query_row(
            "SELECT 1 FROM ref_versions WHERE name = ?1 AND content_hash = ?2",
            params![name, hash],
            |r| r.get(0),
        )
        .optional()?;
    if known.is_some() {
        println!("{name} : déjà chargé, ignoré");
        return Ok(());
    }

    let text = decode_text(&bytes);
    let archive_name = format!("{name}_{}.zst", &hash[..8]);
    let apath = archive::write(&archive_dir.join("ref"), &archive_name, &bytes)?;

    let res: Result<(usize, usize)> = if name == FILE_CIS {
        let p = parse_cis(&text);
        load_cis(conn, &p, &hash, &archive_name).map(|_| (p.rows.len(), p.rejects.len()))
    } else {
        let p = parse_gener(&text);
        load_gener(conn, &p, &hash, &archive_name).map(|_| (p.rows.len(), p.rejects.len()))
    };

    match res {
        Ok((n, r)) => {
            println!("{name} -> {archive_name} : {n} lignes, {r} rejetées");
            Ok(())
        }
        Err(e) => {
            let _ = std::fs::remove_file(&apath);
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodage() {
        assert_eq!(decode_text(&[b'c', 0x9C, b'u', b'r']), "cœur");
        assert_eq!(decode_text(&[b'p', 0xE9]), "pé");
        assert_eq!(decode_text("pellicul\u{e9}".as_bytes()), "pelliculé");
    }

    #[test]
    fn cis() {
        let t = [
            "61266250\tA 313 200 000 UI POUR CENT, pommade\tpommade\tcutanée\tAutorisation active\tProcédure nationale\tCommercialisée\t12/03/1998\t\t\t PHARMA DEVELOPPEMENT\tNon",
            "68257528\tABACAVIR/LAMIVUDINE ACCORD 600 mg/300 mg, comprimé pelliculé\tcomprimé pelliculé\torale\tAutorisation archivée\tProcédure nationale\tNon commercialisée\t16/03/2017\tWarning disponibilité\t\t ACCORD HEALTHCARE FRANCE\tNon",
            "abc\tmauvaise ligne",
        ]
        .join("\n");
        let p = parse_cis(&t);
        assert_eq!(p.rows.len(), 2);
        assert_eq!(p.rejects.len(), 1);
        assert_eq!(p.rows[0].cis, "61266250");
        assert_eq!(p.rows[0].date_amm.as_deref(), Some("1998-03-12"));
        assert_eq!(p.rows[0].titulaire, "PHARMA DEVELOPPEMENT");
        assert_eq!(p.rows[0].statut_bdm, None);
        assert!(!p.rows[0].surveillance_renforcee);
        assert_eq!(p.rows[1].statut_bdm.as_deref(), Some("Warning disponibilité"));
        assert_eq!(p.rows[1].etat_commercialisation, "Non commercialisée");
    }

    #[test]
    fn gener() {
        let t = [
            "1\tCIMETIDINE 200 mg - TAGAMET 200 mg, comprimé pelliculé\t65383183\t0\t1",
            "1\tCIMETIDINE 200 mg - TAGAMET 200 mg, comprimé pelliculé\t67535309\t1\t2",
            "4\tCIMETIDINE 800 mg - TAGAMET 800 mg, comprimé pelliculé sécable\t62844636\t2\t3",
        ]
        .join("\n");
        let p = parse_gener(&t);
        assert_eq!(p.rows.len(), 3);
        assert!(p.rejects.is_empty());
        assert_eq!((p.rows[0].groupe_id, p.rows[0].type_gener), (1, 0));
        assert_eq!((p.rows[2].groupe_id, p.rows[2].type_gener, p.rows[2].cis.as_str()), (4, 2, "62844636"));
    }
}
