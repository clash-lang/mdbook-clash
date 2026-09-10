use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

pub(crate) fn display(command: &[String], args: &[String]) -> String {
    command
        .iter()
        .chain(args)
        .map(|arg| shell_words::quote(arg).into_owned())
        .collect::<Vec<_>>()
        .join(" ")
}

pub(crate) fn run(command: &[String], args: &[String]) -> std::io::Result<Output> {
    let (program, prefix) = command
        .split_first()
        .expect("commands are validated when configuration is loaded");
    Command::new(program).args(prefix).args(args).output()
}

pub(crate) fn run_in_dir(command: &[String], args: &[String], working_directory: PathBuf) -> std::io::Result<Output> {
    let (program, prefix) = command
        .split_first()
        .expect("commands are validated when configuration is loaded");
    Command::new(program).args(prefix).args(args).current_dir(working_directory).output()
}

pub(crate) fn run_and_display(command: &[String], args: &[String], dir: Option<PathBuf>) -> std::io::Result<Output> {
    let display = display(command, args);
    log::info!("running {display}");
    match dir {
        Some(dir) => run_in_dir(command, args, dir),
        None => run(command, args),
    }
}

pub(crate) fn check(output: Output, action: &str, location: &str) -> Result<()> {
    if output.status.success() {
        return Ok(());
    }

    anyhow::bail!(
        "{action} failed at {location}\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

pub(crate) fn fingerprint(command: &[String]) -> Result<String> {
    let program = Path::new(&command[0]);
    let executable = if program.components().count() > 1 {
        program.to_path_buf()
    } else {
        find_on_path(program)
            .with_context(|| format!("{} was not found on PATH", program.display()))?
    };
    let contents = fs::read(&executable)
        .with_context(|| format!("failed to read {}", executable.display()))?;
    Ok(blake3::hash(&contents).to_hex().to_string())
}

fn find_on_path(program: &Path) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|path| {
        std::env::split_paths(&path)
            .map(|directory| directory.join(program))
            .find(|candidate| candidate.is_file())
    })
}
