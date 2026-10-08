use crate::config::Config;
use crate::markdown::{BlockKind, HasGroup};
use crate::{doctest, markdown, netlist, shockwaves, source, synthesis};
use anyhow::Result;
use mdbook_preprocessor::book::{Book, BookItem, Chapter};
use std::path::Path;

pub(crate) struct Processor {
    pub config: Config,
}

impl Processor {
    pub fn run(&self, book: &mut Book) -> Result<()> {
        for item in &mut book.items {
            if let BookItem::Chapter(chapter) = item {
                self.chapter(chapter)?;
            }
        }
        Ok(())
    }

    fn chapter(&self, chapter: &mut Chapter) -> Result<()> {
        let path = chapter
            .path
            .as_deref()
            .unwrap_or_else(|| Path::new("<unknown chapter>"));
        let path = self.config.chapter_path(path);
        let display_path = self.config.display_path(&path);
        let blocks = markdown::blocks(&chapter.content, &display_path)?;

        let mut edits = Vec::new();
        let mut listings = Vec::new();
        for unit in markdown::units(&blocks) {
            doctest::run(&self.config, &path, &unit)?;
            let definitions = source::assemble(&unit);
            let listing_link = unit[0].get_group().map(|group| {
                let anchor = format!("mdbook-clash-listing-{}", listings.len() + 1);
                listings.push(source::listing(&anchor, group, &definitions));
                format!("\n\n[View full listing](#{anchor})\n")
            });

            for block in unit {
                let BlockKind::Code(code_block) = &block.kind else { continue; };

                let addition = if let Some(top_entity) = code_block.attrs.top_entity.as_deref() {
                    let output =
                        synthesis::run(&self.config, &path, block, &definitions, top_entity)?;
                    if code_block.attrs.netlistsvg {
                        netlist::run(&self.config, &path, block, &output)?
                    } else {
                        String::new()
                    }
                } else {
                    String::new()
                };
                if code_block.attrs.hidden {
                    edits.push((block.start, block.end, addition));
                } else {
                    let replacement = format!(
                        "{}{}",
                        listing_link.as_deref().unwrap_or_default(),
                        addition
                    );
                    if !replacement.is_empty() {
                        edits.push((block.end, block.end, replacement));
                    }
                }
                // if let Some(shockwaves) = &code_block.attrs.shockwaves {
                //     dbg!(&code_block.attrs.surfer_batch);
                //     let output = shockwaves::run(
                //         &self.config,
                //         &path,
                //         block,
                //         shockwaves,
                //         &block.attrs.surfer_batch,
                //         &definitions,
                //     )?;
                //     let rendered_md = shockwaves::markdown(&self.config, &output);
                //     edits.push((block.end, block.end, rendered_md));
                // }
            }
        }

        edits.sort_by_key(|edit| edit.0);
        for (start, end, replacement) in edits.into_iter().rev() {
            chapter.content.replace_range(start..end, &replacement);
        }
        if !listings.is_empty() {
            chapter.content.push_str("\n\n## Full code listings\n");
            for listing in listings {
                chapter.content.push_str(&listing);
            }
        }

        for item in &mut chapter.sub_items {
            if let BookItem::Chapter(chapter) = item {
                self.chapter(chapter)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::ClashPreprocessor;
    use mdbook_preprocessor::Preprocessor;

    #[test]
    fn supports_only_html() {
        let preprocessor = ClashPreprocessor;
        assert!(preprocessor.supports_renderer("html").unwrap());
        assert!(!preprocessor.supports_renderer("markdown").unwrap());
    }
}
