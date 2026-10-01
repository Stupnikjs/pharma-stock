use std::path::PathBuf;

/// Emplacements des données (relatifs au dossier courant par défaut).
#[derive(Debug, Clone)]
pub struct Config {
    /// Base SQLite.
    pub db: PathBuf,
    /// Exports SmartRx (.txt) à ingérer ; jamais modifiés.
    pub raw: PathBuf,
    /// Archives zstd (exports et référentiel).
    pub archive: PathBuf,
    /// Fichiers BDPM (CIS_bdpm, CIS_GENER_bdpm, CIS_CIP_bdpm).
    pub refdir: PathBuf,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            db: "data/stock.db".into(),
            raw: "data/raw".into(),
            archive: "data/archive".into(),
            refdir: "data/ref".into(),
        }
    }
}
