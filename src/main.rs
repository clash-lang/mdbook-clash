use anyhow::{bail, Context, Result};
use log::{debug, error};
use mdbook_clash::ClashPreprocessor;
use mdbook_preprocessor::book::Book;
use mdbook_preprocessor::{Preprocessor, PreprocessorContext};
use std::env;
use std::io::{self, Read};

fn main() -> Result<()> {
    env_logger::Builder::from_default_env()
        .filter_level(log::LevelFilter::Info)
        .init();

    let preprocessor = ClashPreprocessor;
    match env::args().nth(1).as_deref() {
        Some("supports") => {
            let renderer = env::args()
                .nth(2)
                .context("missing renderer for supports")?;
            if preprocessor.supports_renderer(&renderer)? {
                return Ok(());
            }
            std::process::exit(1);
        }
        Some("preprocess") => debug!("mdbook-clash preprocess command called"),
        Some(argument) => bail!("unknown argument `{argument}`"),
        None => {}
    }

    let mut input = String::new();
    io::stdin()
        .read_to_string(&mut input)
        .context("reading stdin")?;
    if input.trim().is_empty() {
        bail!("empty stdin for preprocess");
    }

    let (ctx, book): (PreprocessorContext, Book) =
        serde_json::from_str(&input).context("parsing ctx/book JSON")?;

    let out = preprocessor.run(&ctx, book).map_err(|err| {
        error!("Preprocessing failed: {err}");
        anyhow::anyhow!("{err}")
    })?;

    println!("{}", serde_json::to_string(&out)?);
    Ok(())
}
