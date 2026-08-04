use crate::config::Config;
use anyhow::{Context, Result};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

const SCHEMA: u32 = 2;

#[derive(Debug, Eq, PartialEq, Serialize, Deserialize)]
struct Manifest {
    schema: u32,
    version: String,
    phase: String,
    key: String,
    files: Vec<CachedFile>,
}

#[derive(Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
struct CachedFile {
    path: String,
    hash: String,
}

pub(crate) fn key(config: &Config, phase: &str, inputs: serde_json::Value) -> Result<String> {
    let executable = std::env::current_exe().context("failed to locate mdbook-clash")?;
    let implementation = fs::read(&executable)
        .with_context(|| format!("failed to read {}", executable.display()))?;
    let data = serde_json::to_vec(&serde_json::json!({
        "schema": SCHEMA,
        "version": env!("CARGO_PKG_VERSION"),
        "implementation": blake3::hash(&implementation).to_hex().to_string(),
        "user_key": config.cache_key,
        "phase": phase,
        "inputs": inputs,
    }))?;
    Ok(blake3::hash(&data).to_hex().to_string())
}

pub(crate) fn directory(config: &Config, phase: &str, key: &str) -> PathBuf {
    let root = if config.cache {
        config.work_dir.join(format!("cache-v{SCHEMA}"))
    } else {
        config.run_dir.clone()
    };
    root.join(phase).join(key)
}

pub(crate) fn lock(directory: &Path) -> Result<fs::File> {
    let parent = directory.parent().expect("cache entry has a parent");
    fs::create_dir_all(parent).with_context(|| format!("failed to create {}", parent.display()))?;
    let lock_path = directory.with_extension("lock");
    let file = fs::File::create(&lock_path)
        .with_context(|| format!("failed to create {}", lock_path.display()))?;
    file.lock_exclusive()
        .with_context(|| format!("failed to lock {}", lock_path.display()))?;
    Ok(file)
}

pub(crate) fn hit(config: &Config, directory: &Path, phase: &str, key: &str) -> Result<bool> {
    if !config.cache || !directory.exists() {
        return Ok(false);
    }

    let validate = || -> Result<bool> {
        let marker = directory.join("cache.json");
        let actual: Manifest = serde_json::from_slice(&fs::read(&marker)?)?;
        let expected = Manifest {
            schema: SCHEMA,
            version: env!("CARGO_PKG_VERSION").to_string(),
            phase: phase.to_string(),
            key: key.to_string(),
            files: cached_files(directory)?,
        };
        Ok(actual == expected)
    };
    match validate() {
        Ok(true) => Ok(true),
        Ok(false) => {
            log::warn!("discarding invalid cache entry at {}", directory.display());
            Ok(false)
        }
        Err(error) => {
            log::warn!(
                "discarding unreadable cache entry at {}: {error:#}",
                directory.display()
            );
            Ok(false)
        }
    }
}

pub(crate) fn reset(directory: &Path) -> Result<()> {
    if directory.exists() {
        fs::remove_dir_all(directory)
            .with_context(|| format!("failed to remove {}", directory.display()))?;
    }
    fs::create_dir_all(directory)
        .with_context(|| format!("failed to create {}", directory.display()))
}

pub(crate) fn commit(config: &Config, directory: &Path, phase: &str, key: &str) -> Result<()> {
    if !config.cache {
        return Ok(());
    }
    let manifest = Manifest {
        schema: SCHEMA,
        version: env!("CARGO_PKG_VERSION").to_string(),
        phase: phase.to_string(),
        key: key.to_string(),
        files: cached_files(directory)?,
    };
    let marker = directory.join("cache.json");
    fs::write(&marker, serde_json::to_vec_pretty(&manifest)?)
        .with_context(|| format!("failed to write {}", marker.display()))?;
    Ok(())
}

pub(crate) fn directory_hash(root: &Path) -> Result<String> {
    let contents = serde_json::to_vec(&cached_files(root)?)?;
    Ok(blake3::hash(&contents).to_hex().to_string())
}

pub(crate) fn files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut directories = vec![root.to_path_buf()];
    let mut files = Vec::new();
    while let Some(directory) = directories.pop() {
        for entry in fs::read_dir(&directory)
            .with_context(|| format!("failed to read {}", directory.display()))?
        {
            let path = entry?.path();
            if path.is_dir() {
                directories.push(path);
            } else {
                files.push(path);
            }
        }
    }
    files.sort();
    Ok(files)
}

fn cached_files(root: &Path) -> Result<Vec<CachedFile>> {
    let mut files = files(root)?
        .into_iter()
        .filter(|path| path.file_name().and_then(|name| name.to_str()) != Some("cache.json"))
        .map(|path| {
            let contents =
                fs::read(&path).with_context(|| format!("failed to read {}", path.display()))?;
            Ok(CachedFile {
                path: path
                    .strip_prefix(root)
                    .expect("cached file is below its root")
                    .to_string_lossy()
                    .into_owned(),
                hash: blake3::hash(&contents).to_hex().to_string(),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    files.sort();
    Ok(files)
}
