use crate::{
    cache, command,
    config::Config,
    markdown::{Block, ShockwavesAttributes},
    source,
};
use anyhow::{Context, Result};
use std::{fs, path::Path};

#[derive(Debug)]
pub(crate) struct OutputImage {
    pub file_path: String,
    pub resolution: (u32, u32),
}

impl Default for OutputImage {
    fn default() -> Self {
        Self {
            file_path: "images/render.png".to_string(),
            resolution: (800, 200),
        }
    }
}

impl OutputImage {
    pub fn markdown(&self, config: &Config, cache_key: &str) -> String {
        let directory = cache::directory(config, "shockwaves", cache_key);
        let image_path = directory.join(&self.file_path);
        format!(
            r#"<img src="{}" width="{}" height="{}">"#,
            image_path.display().to_string(),
            self.resolution.0,
            self.resolution.1
        )
    }
}

#[derive(Debug)]
pub(crate) struct Output {
    pub cache_key: String,
    pub images: Vec<OutputImage>,
}

pub(crate) fn run(
    config: &Config,
    chapter: &Path,
    block: &Block,
    shockwaves: &ShockwavesAttributes,
    source_text: &str,
) -> Result<Output> {
    let cache_key = cache::key(
        config,
        "shockwaves",
        serde_json::json!({
            "source": source_text,
            "shockwave": shockwaves,
            "clash": config.clash_cmd,
            "clash_fingerprint": command::fingerprint(&config.clash_cmd)?,
            "clash_args": config.clash_args,
            "surfer": config.clash_cmd,
            "surfer_fingerprint": command::fingerprint(&config.surfer_cmd)?,
        }),
    )?;

    let directory = cache::directory(config, "shockwaves", &cache_key);
    let _lock = cache::lock(&directory)?;
    let bin_dir = directory.join("bin");
    let shockwaves_vcd = bin_dir.join("waveform.vcd");
    let shockwaves_json = bin_dir.join("waveform.json");
    let image_dir = directory.join("images");
    let output_image = image_dir.join("render.png");

    if cache::hit(config, &directory, "shockwaves", &cache_key)? {
        log::info!(
            "shockwaves cache hit for {}:{}",
            config.display_path(chapter),
            block.line
        );
        return Ok(Output {
            cache_key,
            images: vec![OutputImage::default()],
        });
    }
    cache::reset(&directory)?;

    let module = source::module_name(source_text);
    let module_path = source::module_path(&directory.join("src"), &module);
    log::info!("Module path {module_path:?} (module: {module:?})");
    fs::create_dir_all(module_path.parent().expect("module has a parent"))
        .with_context(|| format!("failed to create source directory for {module}"))?;
    fs::create_dir_all(&bin_dir)
        .with_context(|| format!("failed to create {}", bin_dir.display()))?;
    fs::create_dir_all(&image_dir)
        .with_context(|| format!("failed to create image directory {}", image_dir.display()))?;
    fs::write(&module_path, source_text)
        .with_context(|| format!("failed to write {}", module_path.display()))?;

    let bin_exec = bin_dir.join(&module);

    let mut clash_args = config.clash_args.clone();
    clash_args.extend([
        module_path.display().to_string(),
        "-main-is".to_string(),
        module,
        "-outputdir".to_string(),
        bin_dir.display().to_string(),
        "-o".to_string(),
        bin_exec.display().to_string(),
    ]);
    let surfer_args = vec![
        "-C".to_string(),
        format!(
            "load_file {}; scope_add logic; zoom_fit; save_image {}x{} {}",
            shockwaves_vcd.display().to_string(),
            shockwaves.resolution.0,
            shockwaves.resolution.1,
            output_image.display().to_string()
        ),
        "oneshot".to_string(),
    ];

    command::run_and_display(&config.clash_cmd, &clash_args, None)?;
    command::run_and_display(&vec![bin_exec.display().to_string()], &[], Some(bin_dir))?;
    command::run_and_display(&config.surfer_cmd, &surfer_args, None)?;

    cache::commit(config, &directory, "shockwaves", &cache_key)?;
    Ok(Output {
        cache_key,
        images: vec![OutputImage::default()],
    })
}

pub(crate) fn markdown(config: &Config, output: &Output) -> String {
    let cache_key = &output.cache_key;
    output
        .images
        .iter()
        .map(|image| image.markdown(config, cache_key))
        .collect::<Vec<_>>()
        .join("\n\n")
}
