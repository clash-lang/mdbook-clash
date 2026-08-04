use anyhow::{bail, Context, Result};
use mdbook_preprocessor::PreprocessorContext;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static RUN_ID: AtomicU64 = AtomicU64::new(0);

#[derive(Debug)]
pub(crate) struct Config {
    pub root: PathBuf,
    pub book_source: PathBuf,
    pub work_dir: PathBuf,
    pub run_dir: PathBuf,
    pub keep_artifacts: bool,
    pub cache: bool,
    pub cache_key: String,
    pub clash_cmd: Vec<String>,
    pub clash_args: Vec<String>,
    pub doctest_cmd: Vec<String>,
    pub yosys_cmd: Vec<String>,
    pub netlistsvg_cmd: Vec<String>,
}

impl Config {
    pub fn from_context(ctx: &PreprocessorContext, doctest_cmd: Vec<String>) -> Result<Self> {
        let work_dir = ctx
            .config
            .get::<String>("preprocessor.clash.work-dir")
            .context("invalid preprocessor.clash.work-dir")?
            .unwrap_or_else(|| "mdbook-clash-work".to_string());
        let relative_work_dir = Path::new(&work_dir);
        if work_dir.is_empty()
            || relative_work_dir.is_absolute()
            || relative_work_dir.components().any(|component| {
                matches!(
                    component,
                    std::path::Component::ParentDir
                        | std::path::Component::RootDir
                        | std::path::Component::Prefix(_)
                )
            })
        {
            bail!("preprocessor.clash.work-dir must be a non-empty relative path");
        }

        let clash_cmd = command(ctx, "clash-cmd", "clash")?;
        let yosys_cmd = command(ctx, "yosys-cmd", "yosys")?;
        let netlistsvg_cmd = command(ctx, "netlistsvg-cmd", "netlistsvg")?;
        if doctest_cmd.is_empty() {
            bail!("doctest command must not be empty");
        }

        let work_dir = ctx.root.join(relative_work_dir);
        let run_dir = work_dir.join("runs").join(format!(
            "{}-{}",
            std::process::id(),
            RUN_ID.fetch_add(1, Ordering::Relaxed)
        ));

        Ok(Self {
            root: ctx.root.clone(),
            book_source: ctx.config.book.src.clone(),
            work_dir,
            run_dir,
            keep_artifacts: ctx
                .config
                .get::<bool>("preprocessor.clash.keep-artifacts")
                .context("invalid preprocessor.clash.keep-artifacts")?
                .unwrap_or(false),
            cache: ctx
                .config
                .get::<bool>("preprocessor.clash.cache")
                .context("invalid preprocessor.clash.cache")?
                .unwrap_or(true),
            cache_key: ctx
                .config
                .get::<String>("preprocessor.clash.cache-key")
                .context("invalid preprocessor.clash.cache-key")?
                .unwrap_or_default(),
            clash_cmd,
            clash_args: ctx
                .config
                .get::<Vec<String>>("preprocessor.clash.clash-args")
                .context("invalid preprocessor.clash.clash-args")?
                .unwrap_or_default(),
            doctest_cmd,
            yosys_cmd,
            netlistsvg_cmd,
        })
    }

    pub fn chapter_path(&self, path: &Path) -> PathBuf {
        if path.is_absolute() || path == Path::new("<unknown chapter>") {
            return path.to_path_buf();
        }
        if path.starts_with(&self.book_source) {
            self.root.join(path)
        } else {
            self.root.join(&self.book_source).join(path)
        }
    }

    pub fn display_path(&self, path: &Path) -> String {
        let cwd = std::env::var_os("PWD")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .or_else(|| std::env::current_dir().ok());
        if let Some(cwd) = cwd {
            if let Ok(relative) = path.strip_prefix(cwd) {
                if !relative.as_os_str().is_empty() {
                    return relative.display().to_string();
                }
            }
        }
        path.display().to_string()
    }
}

fn command(ctx: &PreprocessorContext, key: &str, default: &str) -> Result<Vec<String>> {
    let full_key = format!("preprocessor.clash.{key}");
    let value = ctx
        .config
        .get::<Vec<String>>(&full_key)
        .with_context(|| format!("invalid {full_key}"))?
        .unwrap_or_else(|| vec![default.to_string()]);
    if value.is_empty() {
        bail!("{full_key} must not be empty");
    }
    Ok(value)
}
