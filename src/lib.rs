mod cache;
mod command;
mod config;
mod doctest;
mod markdown;
mod netlist;
mod processor;
mod source;
mod synthesis;

use anyhow::Result;
use config::Config;
use mdbook_preprocessor::book::Book;
use mdbook_preprocessor::errors::Error;
use mdbook_preprocessor::{Preprocessor, PreprocessorContext};
use processor::Processor;
use std::fs;

const DOCTEST_PROGRAM: &str = "mdbook-clash-doctest";

pub struct ClashPreprocessor;

impl ClashPreprocessor {
    #[doc(hidden)]
    pub fn run_with_test_doctest_command(
        &self,
        ctx: &PreprocessorContext,
        book: Book,
        doctest_cmd: Vec<String>,
    ) -> Result<Book, Error> {
        run(ctx, book, doctest_cmd)
    }
}

impl Preprocessor for ClashPreprocessor {
    fn name(&self) -> &str {
        "mdbook-clash"
    }

    fn supports_renderer(&self, renderer: &str) -> Result<bool> {
        Ok(renderer == "html")
    }

    fn run(&self, ctx: &PreprocessorContext, book: Book) -> Result<Book, Error> {
        run(ctx, book, vec![DOCTEST_PROGRAM.to_string()])
    }
}

fn run(ctx: &PreprocessorContext, mut book: Book, doctest_cmd: Vec<String>) -> Result<Book> {
    let config = Config::from_context(ctx, doctest_cmd)?;
    let processor = Processor { config };
    let result = processor.run(&mut book);
    let cleanup = if !processor.config.keep_artifacts && processor.config.run_dir.exists() {
        fs::remove_dir_all(&processor.config.run_dir)
    } else {
        Ok(())
    };
    result?;
    cleanup?;
    Ok(book)
}
