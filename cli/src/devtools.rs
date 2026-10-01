//! Outils de développement (pas de logique métier).
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

fn collect(dir: &Path, ext: &str, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let p = entry?.path();
        if p.is_dir() {
            collect(&p, ext, out)?;
        } else if p.extension().is_some_and(|x| x == ext) {
            out.push(p);
        }
    }
    Ok(())
}

/// Concatène Cargo.toml (racine + crates) et tous les .rs des crates dans
/// `forllm.txt`, à la racine du workspace.
pub fn dump_sources() -> Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .context("racine du workspace introuvable")?;

    let mut files = vec![root.join("Cargo.toml")];
    let mut crates: Vec<PathBuf> = std::fs::read_dir(root.join("crates"))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_dir())
        .collect();
    crates.sort();
    for c in crates {
        files.push(c.join("Cargo.toml"));
        let mut rs = Vec::new();
        collect(&c.join("src"), "rs", &mut rs)?;
        rs.sort();
        files.extend(rs);
    }

    let mut out = String::new();
    for f in files.iter().filter(|f| f.is_file()) {
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
