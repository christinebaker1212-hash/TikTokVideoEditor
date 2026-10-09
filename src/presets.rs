//! JSON preset storage: UI skins, blend presets and projects.

use anyhow::{bail, Result};
use serde::{de::DeserializeOwned, Serialize};
use std::path::PathBuf;

use crate::paths;
use crate::project::{Blend, Skin};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Skins,
    Blends,
    Projects,
}

pub fn dir(kind: Kind) -> PathBuf {
    let d = match kind {
        Kind::Skins => paths::presets_dir().join("skins"),
        Kind::Blends => paths::presets_dir().join("blends"),
        Kind::Projects => paths::workspace().join("projects"),
    };
    let _ = std::fs::create_dir_all(&d);
    d
}

pub fn slug(name: &str) -> Result<String> {
    let s: String = name
        .trim()
        .chars()
        .filter(|c| c.is_alphanumeric() || "._ -".contains(*c))
        .map(|c| if c == ' ' { '_' } else { c })
        .take(80)
        .collect();
    if s.is_empty() || s.starts_with('.') {
        bail!("Invalid preset name");
    }
    Ok(s)
}

pub fn path(kind: Kind, name: &str) -> Result<PathBuf> {
    Ok(dir(kind).join(format!("{}.json", slug(name)?)))
}

/// Names of saved presets (the built-in "Default" is always first for skins/blends).
pub fn list(kind: Kind) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir(kind))
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
                .filter_map(|e| {
                    let text = std::fs::read_to_string(e.path()).ok()?;
                    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
                    Some(v["name"].as_str().map(str::to_string).unwrap_or_else(|| {
                        e.path().file_stem().unwrap_or_default().to_string_lossy().into_owned()
                    }))
                })
                .collect()
        })
        .unwrap_or_default();
    names.sort_by_key(|n| n.to_lowercase());
    if kind != Kind::Projects && !names.iter().any(|n| n == "Default") {
        names.insert(0, "Default".into());
    }
    names
}

pub fn load<T: DeserializeOwned>(kind: Kind, name: &str) -> Result<T> {
    let p = path(kind, name)?;
    if p.exists() {
        return Ok(serde_json::from_str(&std::fs::read_to_string(p)?)?);
    }
    if name == "Default" {
        let v = match kind {
            Kind::Skins => serde_json::to_value(Skin::default())?,
            Kind::Blends => serde_json::to_value(Blend::default())?,
            Kind::Projects => bail!("No project named {name}"),
        };
        return Ok(serde_json::from_value(v)?);
    }
    bail!("No preset named {name}")
}

pub fn save<T: Serialize>(kind: Kind, name: &str, value: &T) -> Result<()> {
    let mut v = serde_json::to_value(value)?;
    v["name"] = serde_json::Value::String(name.to_string());
    std::fs::write(path(kind, name)?, serde_json::to_string_pretty(&v)?)?;
    Ok(())
}

pub fn delete(kind: Kind, name: &str) -> Result<()> {
    let p = path(kind, name)?;
    if p.exists() {
        std::fs::remove_file(p)?;
    }
    Ok(())
}
