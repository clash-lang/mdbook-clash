use crate::cache;
use crate::command;
use crate::config::Config;
use crate::markdown::Block;
use crate::source;
use anyhow::{bail, Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub(crate) struct Output {
    pub verilog_dir: PathBuf,
    pub top_component: String,
    pub cache_key: String,
    pub output_hash: String,
}

pub(crate) fn run(
    config: &Config,
    chapter: &Path,
    block: &Block,
    source_text: &str,
    top_entity: &str,
) -> Result<Output> {
    let key = cache::key(
        config,
        "synth",
        serde_json::json!({
            "source": source_text,
            "top_entity": top_entity,
            "command": config.clash_cmd,
            "command_fingerprint": command::fingerprint(&config.clash_cmd)?,
            "args": config.clash_args,
            "hdl": "verilog",
        }),
    )?;
    let directory = cache::directory(config, "synth", &key);
    let _lock = cache::lock(&directory)?;
    let verilog_dir = directory.join("verilog");

    if cache::hit(config, &directory, "synth", &key)? {
        log::info!(
            "synthesis cache hit for {}:{}",
            config.display_path(chapter),
            block.line
        );
        return result(verilog_dir, key);
    }
    cache::reset(&directory)?;

    let module = source::module_name(source_text);
    let module_path = source::module_path(&directory.join("src"), &module);
    fs::create_dir_all(module_path.parent().expect("module has a parent"))
        .with_context(|| format!("failed to create source directory for {module}"))?;
    fs::create_dir_all(&verilog_dir)
        .with_context(|| format!("failed to create {}", verilog_dir.display()))?;
    fs::write(&module_path, source_text)
        .with_context(|| format!("failed to write {}", module_path.display()))?;

    let mut args = config.clash_args.clone();
    args.extend([
        module_path.display().to_string(),
        "--verilog".to_string(),
        "-main-is".to_string(),
        top_entity.to_string(),
        "-outputdir".to_string(),
        verilog_dir.display().to_string(),
    ]);
    log::info!("running {}", command::display(&config.clash_cmd, &args));

    let output = command::run(&config.clash_cmd, &args).with_context(|| {
        format!(
            "failed to start Clash at {}:{}:1",
            config.display_path(chapter),
            block.line
        )
    })?;
    let location = format!("{}:{}:1", config.display_path(chapter), block.line);
    if let Err(error) = command::check(output, "synthesis", &location) {
        let _ = fs::remove_dir_all(&directory);
        return Err(error);
    }

    let result = result(verilog_dir, key.clone())?;
    cache::commit(config, &directory, "synth", &key)?;
    Ok(result)
}

fn result(verilog_dir: PathBuf, cache_key: String) -> Result<Output> {
    let manifests = cache::files(&verilog_dir)?
        .into_iter()
        .filter(|path| {
            path.file_name().and_then(|name| name.to_str()) == Some("clash-manifest.json")
        })
        .collect::<Vec<_>>();
    if manifests.len() != 1 {
        bail!(
            "expected one Clash manifest below {}, found {}",
            verilog_dir.display(),
            manifests.len()
        );
    }
    let manifest: serde_json::Value = serde_json::from_slice(
        &fs::read(&manifests[0])
            .with_context(|| format!("failed to read {}", manifests[0].display()))?,
    )?;
    let top_component = manifest["top_component"]["name"]
        .as_str()
        .context("Clash manifest has no top component name")?
        .to_string();
    let output_hash = cache::directory_hash(&verilog_dir)?;
    Ok(Output {
        verilog_dir,
        top_component,
        cache_key,
        output_hash,
    })
}
