use anyhow::{Context, Result};
use std::path::Path;

#[derive(Debug, Clone, PartialEq)]
pub struct Product {
    pub libelle: String,
    pub cip13: String,
    pub lis: Option<String>,
    pub cdt: Option<i64>,
    pub px_rem_ug: Option<f64>,
    pub px_moy_pond: Option<f64>,
    pub rbt: Option<String>,
    pub stock: Option<i64>,
    pub tva: Option<f64>,
    pub pu_vente: Option<f64>,
    pub qte_vendue: Option<i64>,
    // Flags qualité (on ne supprime rien, on marque)
    pub cip_valide: bool,
    pub code_interne: bool,
    pub libelle_vide: bool,
    pub libelle_est_code: bool,
}

#[derive(Debug)]
pub struct Snapshot {
    /// Date ISO (AAAA-MM-JJ) lue dans l'en-tête "Le 14/09/26;"
    pub date: String,
    pub products: Vec<Product>,
}

/// Clé de contrôle EAN13 (les CIP13 valides la respectent).
pub fn ean13_ok(code: &str) -> bool {
    let d: Vec<u32> = code.chars().filter_map(|c| c.to_digit(10)).collect();
    if code.len() != 13 || d.len() != 13 {
        return false;
    }
    let s: u32 = d[..12]
        .iter()
        .enumerate()
        .map(|(i, &x)| if i % 2 == 1 { 3 * x } else { x })
        .sum();
    (10 - s % 10) % 10 == d[12]
}

fn opt_str(s: &str) -> Option<String> {
    let t = s.trim();
    if t.is_empty() {
        None
    } else {
        Some(t.to_string())
    }
}

/// "........" ou vide -> None ; virgule décimale acceptée.
fn parse_f64(s: &str) -> Option<f64> {
    let t = s.trim();
    if t.is_empty() || t.chars().all(|c| c == '.') {
        return None;
    }
    t.replace(',', ".").parse().ok()
}

fn parse_i64(s: &str) -> Option<i64> {
    let t = s.trim();
    if t.is_empty() || t.chars().all(|c| c == '.') {
        return None;
    }
    t.parse().ok()
}

fn parse_date(line: &str) -> Option<String> {
    let rest = line.trim().strip_prefix("Le ")?.trim().trim_end_matches(';').trim();
    let mut it = rest.split('/');
    let d: u32 = it.next()?.trim().parse().ok()?;
    let m: u32 = it.next()?.trim().parse().ok()?;
    let y: u32 = it.next()?.trim().parse().ok()?;
    if it.next().is_some() || !(1..=31).contains(&d) || !(1..=12).contains(&m) {
        return None;
    }
    let y = if y < 100 { 2000 + y } else { y };
    Some(format!("{y:04}-{m:02}-{d:02}"))
}

/// Parse le contenu texte (déjà décodé) d'un export SmartRx.
pub fn parse_snapshot(text: &str) -> Result<Snapshot> {
    let lines: Vec<&str> = text.lines().collect();

    let date = lines
        .iter()
        .take(10)
        .find_map(|l| parse_date(l))
        .context("date du snapshot introuvable dans l'en-tête")?;

    let hdr = lines
        .iter()
        .position(|l| l.trim_start().starts_with("Libell"))
        .context("ligne d'en-tête des colonnes introuvable")?;

    let mut products = Vec::new();
    for line in &lines[hdr + 1..] {
        if line.trim().is_empty() {
            continue;
        }
        let f: Vec<&str> = line.split(';').collect();
        let g = |i: usize| f.get(i).copied().unwrap_or("");

        let libelle = g(0).trim().to_string();
        let cip13: String = g(1).chars().filter(|c| !c.is_whitespace()).collect();

        products.push(Product {
            libelle_vide: libelle.is_empty(),
            libelle_est_code: (8..=14).contains(&libelle.len())
                && libelle.chars().all(|c| c.is_ascii_digit()),
            cip_valide: ean13_ok(&cip13),
            code_interne: cip13.starts_with("20000"),
            lis: opt_str(g(2)),
            cdt: parse_i64(g(3)),
            px_rem_ug: parse_f64(g(4)),
            px_moy_pond: parse_f64(g(5)),
            rbt: opt_str(g(6)),
            stock: parse_i64(g(7)),
            tva: parse_f64(g(8)),
            pu_vente: parse_f64(g(9)),
            qte_vendue: parse_i64(g(10)),
            cip13,
            libelle,
        });
    }

    Ok(Snapshot { date, products })
}

/// Moitié haute (0x80..=0xFF) de la page de code CP850, dans l'ordre.
const CP850_HIGH: &str = concat!(
    "Çüéâäàåçêëèïîì", "ÄÅ",
    "ÉæÆôöòûùÿÖÜø£Ø×ƒ",
    "áíóúñÑªº¿®¬½¼¡«»",
    "░▒▓│┤ÁÂÀ©╣║╗╝¢¥┐",
    "└┴┬├─┼ãÃ╚╔╩╦╠═╬¤",
    "ðÐÊËÈıÍÎÏ┘┌█▄¦Ì▀",
    "ÓßÔÒõÕµþÞÚÛÙýÝ¯´",
    "\u{00AD}±‗¾¶§÷¸°¨·¹³²■\u{00A0}",
);

/// Décode des octets CP850 (export DOS de SmartRx) en String.
pub fn decode_cp850(bytes: &[u8]) -> String {
    let high: Vec<char> = CP850_HIGH.chars().collect();
    bytes
        .iter()
        .map(|&b| if b < 0x80 { b as char } else { high[(b - 0x80) as usize] })
        .collect()
}

/// Lit un fichier .txt SmartRx (encodage CP850).
pub fn read_file(path: &Path) -> Result<Snapshot> {
    let bytes = std::fs::read(path).with_context(|| format!("lecture de {}", path.display()))?;
    let text = decode_cp850(&bytes);
    parse_snapshot(&text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> String {
        [
            "LISTE DES PRODUITS;",
            "ORDRE ALPHABETIQUE;",
            "Le 14/09/26;",
            "EDITION COMPLETE;",
            "Libellé du Produit            ;Code CIP13       ;Lis;Cdt;Px Rem.UG;PxMoy.Pon;Rbt ;St.Réel;Tva;PU vente ;QtéVent;",
            "                              ; 20000 9200019 1 ;   ;  0;........ ;........ ; NR ;     0 ; 4 ;........ ;     0 ;",
            "                              ;                 ;   ;  0;........ ;........ ; NR ;     0 ; 5 ;    2,75 ;     0 ;",
            "100% DESTOCK GELUL 30         ; 34015 4441043 5 ;   ;  0;   15,90 ;   15,90 ; NR ;     3 ; 2 ;   29,45 ;     1 ;",
            "3582910077671                 ; 20000 9200048 1 ;   ;  0;........ ;........ ; NR ;     0 ; 4 ;........ ;     0 ;",
        ]
        .join("\n")
    }

    #[test]
    fn parse_echantillon() {
        let s = parse_snapshot(&sample()).unwrap();
        assert_eq!(s.date, "2026-09-14");
        assert_eq!(s.products.len(), 4);

        let p = &s.products[2];
        assert_eq!(p.cip13, "3401544410435");
        assert!(p.cip_valide && !p.code_interne);
        assert_eq!(p.px_rem_ug, Some(15.9));
        assert_eq!(p.stock, Some(3));
        assert_eq!(p.pu_vente, Some(29.45));

        assert!(s.products[0].code_interne && s.products[0].libelle_vide);
        assert_eq!(s.products[0].pu_vente, None);
        assert_eq!(s.products[1].pu_vente, Some(2.75));
        assert!(!s.products[1].cip_valide);
        assert!(s.products[3].libelle_est_code);
    }

    #[test]
    fn cp850() {
        assert_eq!(CP850_HIGH.chars().count(), 128);
        assert_eq!(decode_cp850(&[b'L', b'i', b'b', 0x82, b'!']), "Libé!");
        assert_eq!(decode_cp850(&[0x8A, 0x85, 0x87]), "èàç");
    }

    #[test]
    fn ean13() {
        assert!(ean13_ok("3401544410435"));
        assert!(!ean13_ok("3401544410436"));
        assert!(!ean13_ok(""));
    }
}
