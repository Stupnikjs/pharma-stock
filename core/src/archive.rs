use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// Empreinte SHA-256 (hex) du contenu brut d'un export.
pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Nom d'archive stable : `AAAA-MM-JJ_NN_<hash8>.txt.zst`
/// (NN = numéro d'ordre du snapshot dans la journée).
pub fn archive_name(date: &str, seq: i64, hash: &str) -> String {
    format!("{date}_{seq:02}_{}.txt.zst", &hash[..8])
}

/// Compresse (zstd) et écrit l'export brut dans le dossier d'archive.
pub fn write(dir: &Path, name: &str, bytes: &[u8]) -> Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join(name);
    let packed = zstd::encode_all(bytes, 19).context("compression zstd")?;
    std::fs::write(&path, packed).with_context(|| format!("écriture de {}", path.display()))?;
    Ok(path)
}

/// Relit une archive : renvoie les octets bruts d'origine (CP850).
pub fn read(path: &Path) -> Result<Vec<u8>> {
    let packed = std::fs::read(path).with_context(|| format!("lecture de {}", path.display()))?;
    zstd::decode_all(&packed[..]).context("décompression zstd")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_vide() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn nom() {
        assert_eq!(
            archive_name("2026-09-14", 1, "a3f9c2d1ffff"),
            "2026-09-14_01_a3f9c2d1.txt.zst"
        );
    }

    #[test]
    fn aller_retour() {
        let dir = std::env::temp_dir().join("pharma_stock_test_archive");
        let data = b"Libell\x82 du Produit;Code CIP13;\n".repeat(100);
        let p = write(&dir, "t.txt.zst", &data).unwrap();
        assert_eq!(read(&p).unwrap(), data);
        let _ = std::fs::remove_file(p);
    }
}
