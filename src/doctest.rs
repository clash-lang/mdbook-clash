use crate::cache;
use crate::command;
use crate::config::Config;
use crate::markdown::Block;
use crate::source;
use anyhow::{Context, Result};
use std::fs;
use std::path::Path;

pub(crate) fn run(config: &Config, chapter: &Path, blocks: &[&Block]) -> Result<()> {
    let tests = blocks
        .iter()
        .copied()
        .filter(|block| !source::doctest(&block.code).transcript.is_empty())
        .collect::<Vec<_>>();
    if tests.is_empty() {
        return Ok(());
    }

    let module_source = source::assemble(blocks);
    let module_name = source::module_name(&module_source);
    let documents = tests
        .iter()
        .map(|block| {
            let document = source::doctest(&block.code);
            serde_json::json!({
                "block": block.index,
                "line": block.line + document.line_offset,
                "transcript": document.transcript,
            })
        })
        .collect::<Vec<_>>();
    let key = cache::key(
        config,
        "test",
        serde_json::json!({
            "source": module_source,
            "module": module_name,
            "documents": documents,
            "doctest": config.doctest_cmd,
            "doctest_fingerprint": command::fingerprint(&config.doctest_cmd)?,
            "clash": config.clash_cmd,
            "clash_fingerprint": command::fingerprint(&config.clash_cmd)?,
            "clash_args": config.clash_args,
        }),
    )?;
    let directory = cache::directory(config, "test", &key);
    let _lock = cache::lock(&directory)?;
    if cache::hit(config, &directory, "test", &key)? {
        log::info!("simulation cache hit for {}", config.display_path(chapter));
        return Ok(());
    }
    cache::reset(&directory)?;

    let source_dir = directory.join("src");
    let module_path = source::module_path(&source_dir, &module_name);
    fs::create_dir_all(module_path.parent().expect("module has a parent"))
        .with_context(|| format!("failed to create {}", source_dir.display()))?;
    fs::write(&module_path, module_source)
        .with_context(|| format!("failed to write {}", module_path.display()))?;

    let mut args = vec![config.clash_cmd.len().to_string()];
    args.extend(config.clash_cmd.iter().cloned());
    args.push(config.clash_args.len().to_string());
    args.extend(config.clash_args.iter().cloned());
    args.push(module_name);
    args.push(module_path.display().to_string());

    for block in tests {
        let document = source::doctest(&block.code);
        let path = directory.join(format!("doctest-{}.txt", block.index));
        fs::write(&path, document.transcript)
            .with_context(|| format!("failed to write {}", path.display()))?;
        args.push(config.display_path(chapter));
        args.push((block.line + document.line_offset).to_string());
        args.push(path.display().to_string());
    }

    log::info!("running {}", command::display(&config.doctest_cmd, &args));
    let output = command::run(&config.doctest_cmd, &args).with_context(|| {
        format!(
            "failed to start doctest at {}",
            config.display_path(chapter)
        )
    })?;
    let location = format!("{}:{}:1", config.display_path(chapter), blocks[0].line);
    if let Err(error) = command::check(output, "doctest", &location) {
        let _ = fs::remove_dir_all(&directory);
        return Err(error);
    }

    cache::commit(config, &directory, "test", &key)?;
    Ok(())
}
