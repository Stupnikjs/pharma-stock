//! Référentiel BDPM : CIS_bdpm.txt, CIS_GENER_bdpm.txt et CIS_CIP_bdpm.txt (CIP13 <-> CIS).
use crate::archive;
use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use std::path::Path;

pub const FILE_CIS: &str = "CIS_bdpm.txt";
pub const FILE_GENER: &str = "CIS_GENER_bdpm.txt";
pub const FILE_CIP: &str = "CIS_CIP_bdpm.txt";

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

#[derive(Debug, Clone, PartialEq)]
pub struct CipRow {
    pub cip13: String,
    pub cip7: String,
    pub cis: String,
    pub libelle_presentation: String,
    pub statut_presentation: String,
    pub etat_commercialisation: String,
    pub date_declaration: Option<String>,
    pub agrement_collectivites: bool,
    /// Taux de remboursement en % (65, 30, 100...), None si non remboursable.
    pub taux_remb: Option<i64>,
    /// Prix hors honoraires, en centimes.
    pub prix_c: Option<i64>,
    /// Prix honoraires de dispensation inclus, en centimes.
    pub prix_hono_c: Option<i64>,
    pub honoraires_c: Option<i64>,
    pub indications_remb: Option<String>,
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

/// "24,34" -> 2434 ; "1,466,29" -> 146629 ; "" -> None.
/// Piège BDPM : au-delà de 999 €, la virgule sert AUSSI de séparateur de milliers
/// (le dernier groupe après une virgule est toujours les centimes).
pub fn prix_cents(s: &str) -> Result<Option<i64>, String> {
    let t = s.trim();
    if t.is_empty() {
        return Ok(None);
    }
    let parts: Vec<&str> = t.split(',').collect();
    let (int_parts, dec): (Vec<&str>, &str) = match parts.split_last() {
        Some((last, head)) if !head.is_empty() => (head.to_vec(), *last),
        _ => (vec![t], ""),
    };
    let int: String = int_parts.concat();
    let digits = |x: &str| x.chars().all(|c| c.is_ascii_digit());
    if int.is_empty() || !digits(&int) || !digits(dec) || dec.len() > 2 {
        return Err(format!("prix illisible : {t}"));
    }
    let cents: i64 = match dec.len() {
        0 => 0,
        1 => dec.parse::<i64>().map_err(|_| format!("prix illisible : {t}"))? * 10,
        _ => dec.parse::<i64>().map_err(|_| format!("prix illisible : {t}"))?,
    };
    let int_v: i64 = int.parse().map_err(|_| format!("prix illisible : {t}"))?;
    Ok(Some(int_v * 100 + cents))
}

/// "65%" / "65 %" -> 65 ; "" -> None.
fn taux_pct(s: &str) -> Result<Option<i64>, String> {
    let t: String = s.chars().filter(|c| !c.is_whitespace() && *c != '%').collect();
    if t.is_empty() {
        return Ok(None);
    }
    t.parse().map(Some).map_err(|_| format!("taux illisible : {s}"))
}

fn cip_line(f: &[&str]) -> Result<CipRow, String> {
    let (cis, cip7, cip13) = (f[0].trim(), f[1].trim(), f[6].trim());
    if !is_cis(cis) {
        return Err("code CIS invalide".into());
    }
    if cip7.len() != 7 || !cip7.bytes().all(|b| b.is_ascii_digit()) {
        return Err("CIP7 invalide".into());
    }
    if !crate::parse::ean13_ok(cip13) {
        return Err("CIP13 invalide (clé de contrôle)".into());
    }
    Ok(CipRow {
        cip13: cip13.to_string(),
        cip7: cip7.to_string(),
        cis: cis.to_string(),
        libelle_presentation: f[2].trim().to_string(),
        statut_presentation: f[3].trim().to_string(),
        etat_commercialisation: f[4].trim().to_string(),
        date_declaration: date_fr(f[5]),
        agrement_collectivites: f[7].trim().eq_ignore_ascii_case("oui"),
        taux_remb: taux_pct(f[8])?,
        prix_c: prix_cents(f[9])?,
        prix_hono_c: prix_cents(f[10])?,
        honoraires_c: prix_cents(f[11])?,
        indications_remb: f.get(12).and_then(|s| opt(s)),
    })
}

pub fn parse_cip(text: &str) -> Parsed<CipRow> {
    let (mut rows, mut rejects) = (Vec::new(), Vec::new());
    for (i, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() < 12 {
            rejects.push(rej(i + 1, line, format!("{} colonnes au lieu de 13", f.len())));
            continue;
        }
        match cip_line(&f) {
            Ok(r) => rows.push(r),
            Err(reason) => rejects.push(rej(i + 1, line, reason)),
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

pub fn load_cip(conn: &mut Connection, p: &Parsed<CipRow>, hash: &str, archive_file: &str) -> Result<()> {
    let tx = conn.transaction()?;
    tx.execute("DELETE FROM bdpm_cip", [])?;
    {
        let mut st = tx.prepare(
            "INSERT OR REPLACE INTO bdpm_cip(cip13, cip7, cis, libelle_presentation,
                statut_presentation, etat_commercialisation, date_declaration,
                agrement_collectivites, taux_remb, prix_c, prix_hono_c, honoraires_c,
                indications_remb)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",
        )?;
        for r in &p.rows {
            st.execute(params![
                r.cip13,
                r.cip7,
                r.cis,
                r.libelle_presentation,
                r.statut_presentation,
                r.etat_commercialisation,
                r.date_declaration,
                r.agrement_collectivites,
                r.taux_remb,
                r.prix_c,
                r.prix_hono_c,
                r.honoraires_c,
                r.indications_remb,
            ])?;
        }
    }
    finish(&tx, FILE_CIP, hash, archive_file, p.rows.len(), &p.rejects)?;
    tx.commit()?;
    Ok(())
}

#[derive(Debug)]
pub enum RefOutcome {
    Loaded { name: &'static str, archive_file: String, lignes: usize, rejets: usize },
    /// Même contenu (empreinte) déjà chargé.
    AlreadyKnown { name: &'static str },
    Failed(String),
    /// Fichier .txt de data/ref qui n'est pas un fichier BDPM attendu.
    Unrecognized,
}

#[derive(Debug)]
pub struct RefFileReport {
    pub file: String,
    pub outcome: RefOutcome,
}

#[derive(Debug, Default)]
pub struct RefReport {
    pub files: Vec<RefFileReport>,
}

/// Charge les fichiers BDPM présents dans `ref_dir` (s'il existe) : archive zstd
/// dans `<archive_dir>/ref/`, puis remplacement complet de la table concernée.
/// Un fichier déjà chargé (même empreinte) est ignoré.
pub fn sync_ref(ref_dir: &Path, archive_dir: &Path, conn: &mut Connection) -> Result<RefReport> {
    let mut report = RefReport::default();
    if !ref_dir.is_dir() {
        return Ok(report);
    }
    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(ref_dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_file() && p.extension().is_some_and(|x| x.eq_ignore_ascii_case("txt")))
        .collect();
    files.sort();

    for path in files {
        let file = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let outcome = match canonical_name(&file) {
            Some(name) => match sync_ref_one(&path, name, archive_dir, conn) {
                Ok(o) => o,
                Err(e) => RefOutcome::Failed(format!("{e:#}")),
            },
            None => RefOutcome::Unrecognized,
        };
        report.files.push(RefFileReport { file, outcome });
    }
    Ok(report)
}

/// Reconnaît un fichier BDPM même renommé par le navigateur
/// ("CIS_CIP_bdpm (1).txt", "CIS_CIP_bdpm__1_.txt"...).
fn canonical_name(file: &str) -> Option<&'static str> {
    let f = file.to_lowercase();
    if f.starts_with("cis_cip_bdpm") {
        Some(FILE_CIP)
    } else if f.starts_with("cis_gener_bdpm") {
        Some(FILE_GENER)
    } else if f.starts_with("cis_bdpm") {
        Some(FILE_CIS)
    } else {
        None
    }
}

fn sync_ref_one(
    path: &Path,
    name: &'static str,
    archive_dir: &Path,
    conn: &mut Connection,
) -> Result<RefOutcome> {
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
        return Ok(RefOutcome::AlreadyKnown { name });
    }

    let text = decode_text(&bytes);
    let archive_name = format!("{name}_{}.zst", &hash[..8]);
    let apath = archive::write(&archive_dir.join("ref"), &archive_name, &bytes)?;

    let res: Result<(usize, usize)> = match name {
        FILE_CIS => {
            let p = parse_cis(&text);
            load_cis(conn, &p, &hash, &archive_name).map(|_| (p.rows.len(), p.rejects.len()))
        }
        FILE_GENER => {
            let p = parse_gener(&text);
            load_gener(conn, &p, &hash, &archive_name).map(|_| (p.rows.len(), p.rejects.len()))
        }
        _ => {
            let p = parse_cip(&text);
            load_cip(conn, &p, &hash, &archive_name).map(|_| (p.rows.len(), p.rejects.len()))
        }
    };

    match res {
        Ok((lignes, rejets)) => Ok(RefOutcome::Loaded { name, archive_file: archive_name, lignes, rejets }),
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
    fn noms_fichiers() {
        assert_eq!(canonical_name("CIS_CIP_bdpm__1_.txt"), Some(FILE_CIP));
        assert_eq!(canonical_name("CIS_CIP_bdpm (1).txt"), Some(FILE_CIP));
        assert_eq!(canonical_name("CIS_GENER_bdpm.txt"), Some(FILE_GENER));
        assert_eq!(canonical_name("CIS_bdpm.txt"), Some(FILE_CIS));
        assert_eq!(canonical_name("autre.txt"), None);
    }

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
    fn prix() {
        assert_eq!(prix_cents("24,34"), Ok(Some(2434)));
        assert_eq!(prix_cents("1,466,29"), Ok(Some(146629)));
        assert_eq!(prix_cents("66,596,25"), Ok(Some(6659625)));
        assert_eq!(prix_cents("12"), Ok(Some(1200)));
        assert_eq!(prix_cents("3,5"), Ok(Some(350)));
        assert_eq!(prix_cents(""), Ok(None));
        assert!(prix_cents("a,12").is_err());
        assert_eq!(taux_pct("65 %"), Ok(Some(65)));
        assert_eq!(taux_pct("100%"), Ok(Some(100)));
        assert_eq!(taux_pct(""), Ok(None));
    }

    #[test]
    fn cip() {
        let t = [
            "60002283\t4949729\tplaquette(s) PVC PVDC aluminium de 30 comprimé(s)\tPrésentation active\tDéclaration de commercialisation\t16/03/2011\t3400949497294\toui\t100%\t24,34\t25,36\t1,02\t",
            "60004505\t5507419\t1 flacon(s) en verre de 1,5 ml\tPrésentation active\tDéclaration d'arrêt de commercialisation\t31/12/2023\t3400955074199\tnon\t\t\t\t\t",
            "60004487\t3972519\tplaquette(s) opaque(s) PVC-Aluminium de 30 comprimé(s)\tPrésentation active\tDéclaration de commercialisation\t12/12/2011\t3400939725192\toui\t65 %\t1,466,29\t1,469,05\t2,76\t",
            "60004487\t3972519\tx\tPrésentation active\tDéclaration de commercialisation\t12/12/2011\t3400939725193\toui\t65 %\t1,00\t2,00\t1,02\t",
        ]
        .join("\n");
        let p = parse_cip(&t);
        assert_eq!(p.rows.len(), 3);
        assert_eq!(p.rejects.len(), 1); // clé EAN13 fausse
        let a = &p.rows[0];
        assert_eq!((a.cip13.as_str(), a.cis.as_str()), ("3400949497294", "60002283"));
        assert_eq!((a.taux_remb, a.prix_c, a.prix_hono_c, a.honoraires_c), (Some(100), Some(2434), Some(2536), Some(102)));
        assert!(a.agrement_collectivites);
        assert_eq!(a.date_declaration.as_deref(), Some("2011-03-16"));
        let b = &p.rows[1];
        assert_eq!((b.taux_remb, b.prix_c), (None, None));
        assert!(!b.agrement_collectivites);
        assert_eq!(p.rows[2].prix_c, Some(146629));
        assert_eq!(p.rows[2].taux_remb, Some(65));
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
