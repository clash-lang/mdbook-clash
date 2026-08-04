use crate::cache;
use crate::command;
use crate::config::Config;
use crate::markdown::Block;
use crate::synthesis;
use anyhow::{bail, Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

pub(crate) fn run(
    config: &Config,
    chapter: &Path,
    block: &Block,
    synthesis: &synthesis::Output,
) -> Result<String> {
    let key = cache::key(
        config,
        "netlistsvg",
        serde_json::json!({
            "synthesis": synthesis.cache_key,
            "synthesis_output": synthesis.output_hash,
            "top_component": synthesis.top_component,
            "yosys_commands": block.attrs.yosys,
            "yosys": config.yosys_cmd,
            "yosys_fingerprint": command::fingerprint(&config.yosys_cmd)?,
            "netlistsvg": config.netlistsvg_cmd,
            "netlistsvg_fingerprint": command::fingerprint(&config.netlistsvg_cmd)?,
        }),
    )?;
    let directory = cache::directory(config, "netlistsvg", &key);
    let _lock = cache::lock(&directory)?;
    let svg_path = directory.join("netlist.svg");
    if cache::hit(config, &directory, "netlistsvg", &key)? {
        log::info!(
            "netlist cache hit for {}:{}",
            config.display_path(chapter),
            block.line
        );
        return markdown(&svg_path);
    }
    cache::reset(&directory)?;

    let verilog = cache::files(&synthesis.verilog_dir)?
        .into_iter()
        .filter(|path| path.extension().and_then(|extension| extension.to_str()) == Some("v"))
        .collect::<Vec<_>>();
    if verilog.is_empty() {
        bail!(
            "no Verilog files found below {}",
            synthesis.verilog_dir.display()
        );
    }

    let json_path = directory.join("netlist.json");
    let script_path = directory.join("netlist.ys");
    fs::write(
        &script_path,
        yosys_script(
            &block.attrs.yosys,
            &verilog,
            &synthesis.top_component,
            &json_path,
        )
        .join("\n")
            + "\n",
    )
    .with_context(|| format!("failed to write {}", script_path.display()))?;

    let yosys_args = vec!["-s".to_string(), script_path.display().to_string()];
    log::info!(
        "running {}",
        command::display(&config.yosys_cmd, &yosys_args)
    );
    let output = command::run(&config.yosys_cmd, &yosys_args).with_context(|| {
        format!(
            "failed to start Yosys at {}:{}:1",
            config.display_path(chapter),
            block.line
        )
    })?;
    let location = format!("{}:{}:1", config.display_path(chapter), block.line);
    if let Err(error) = command::check(output, "Yosys", &location) {
        let _ = fs::remove_dir_all(&directory);
        return Err(error);
    }

    let netlistsvg_args = vec![
        json_path.display().to_string(),
        "-o".to_string(),
        svg_path.display().to_string(),
    ];
    log::info!(
        "running {}",
        command::display(&config.netlistsvg_cmd, &netlistsvg_args)
    );
    let output = command::run(&config.netlistsvg_cmd, &netlistsvg_args).with_context(|| {
        format!(
            "failed to start netlistsvg at {}:{}:1",
            config.display_path(chapter),
            block.line
        )
    })?;
    if let Err(error) = command::check(output, "netlistsvg", &location) {
        let _ = fs::remove_dir_all(&directory);
        return Err(error);
    }

    cache::commit(config, &directory, "netlistsvg", &key)?;
    markdown(&svg_path)
}

fn markdown(path: &Path) -> Result<String> {
    let svg =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    Ok(format!(
        r#"

#### Netlist

<div class="mdbook-clash-netlist" style="background: white; padding: 1rem; overflow-x: auto;">
{svg}
</div>
"#
    ))
}

fn yosys_script(
    user_commands: &[String],
    verilog_files: &[PathBuf],
    top_component: &str,
    json_path: &Path,
) -> Vec<String> {
    let quote = |value: &str| format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""));
    let verilog = verilog_files
        .iter()
        .map(|path| quote(&path.display().to_string()))
        .collect::<Vec<_>>()
        .join(" ");
    let mut commands = vec![
        format!("read_verilog {verilog}"),
        format!("hierarchy -top {top_component}"),
    ];
    if user_commands.is_empty() {
        commands.extend(["proc".to_string(), "opt".to_string()]);
    } else {
        commands.extend_from_slice(user_commands);
    }
    commands.push("clean -purge".to_string());
    commands.push(format!(
        "write_json {}",
        quote(&json_path.display().to_string())
    ));
    commands
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_yosys_script() {
        let files = vec![PathBuf::from("/tmp/topEntity.v")];
        let user = vec!["proc".to_string(), "opt".to_string(), "techmap".to_string()];
        let script = yosys_script(&user, &files, "adder", Path::new("/tmp/netlist.json"));
        assert!(script[0].starts_with("read_verilog "));
        assert!(script.contains(&"hierarchy -top adder".to_string()));
        assert!(script.contains(&"techmap".to_string()));
        assert_eq!(
            script.last().map(String::as_str),
            Some("write_json \"/tmp/netlist.json\"")
        );
    }
}
