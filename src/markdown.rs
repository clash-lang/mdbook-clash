use crate::builder_gen::*;
use anyhow::{anyhow, bail, Context, Result};
use paste::paste;
use pulldown_cmark::{CodeBlockKind, Event, Parser, Tag, TagEnd};
use serde::Serialize;
use std::collections::HashMap;

#[derive(Clone, Debug)]
pub(crate) struct CodeBlock {
    pub attrs: Attributes,
    pub code: String,
}

#[derive(Clone, Debug)]
pub(crate) enum BlockKind {
    Code(CodeBlock),
    Surfer(SurferBlock),
}

#[derive(Clone, Debug)]
pub(crate) struct Block {
    pub kind: BlockKind,
    pub index: usize,
    pub line: usize,
    pub start: usize,
    pub end: usize,
}

pub(crate) trait HasGroup {
    fn get_group(&self) -> Option<&str>;
}

impl HasGroup for CodeBlock {
    fn get_group(&self) -> Option<&str> {
        self.attrs.group.as_deref()
    }
}

impl HasGroup for SurferBlock {
    fn get_group(&self) -> Option<&str> {
        Some(self.group.as_str())
    }
}

impl HasGroup for BlockKind {
    fn get_group(&self) -> Option<&str> {
        match self {
            BlockKind::Code(code_block) => code_block.get_group(),
            BlockKind::Surfer(surfer_block) => surfer_block.get_group(),
        }
    }
}

impl HasGroup for Block {
    fn get_group(&self) -> Option<&str> {
        self.kind.get_group()
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Attributes {
    pub group: Option<String>,
    pub top_entity: Option<String>,
    pub yosys: Vec<String>,
    pub netlistsvg: bool,
    pub hidden: bool,
}

auto_builder!(
    SurferBlock <- SurferBlockBuilder (SurferBlockProperty) =
        group (String) required("Do not know from what code block to monitor signals"),
        cycle_start (u32) required("Do not know which cycle to start simulation from"),
        cycle_end (u32) required("Do not know which cycle to end simulation at"),
        width (u32) required("Do not know image width"),
        height (u32) required("Do not know image height"),
        signals (Vec<String>) warn("No signals specified to monitor, an empty image will be generated"),
        surfer_batch (Vec<String>)
);

impl SurferBlockBuilder {
    fn new() -> Self {
        Self::default()
    }
}

#[derive(Debug)]
enum SurferBlockParser {
    None,
    Builder(SurferBlockBuilder),
    Final(SurferBlock),
}

impl SurferBlockParser {
    fn map_builder<F>(self, mapper: F) -> anyhow::Result<Self>
    where
        F: FnOnce(SurferBlockBuilder) -> anyhow::Result<SurferBlockBuilder>,
    {
        match self {
            SurferBlockParser::None => Ok(self),
            SurferBlockParser::Builder(surfer_block_builder) => {
                Ok(Self::Builder(mapper(surfer_block_builder)?))
            }
            SurferBlockParser::Final(_) => Ok(self),
        }
    }
}

pub(crate) fn blocks(content: &str, source: &str) -> Result<Vec<Block>> {
    let mut blocks = Vec::new();
    let mut active: Option<(String, usize, String)> = None;

    let mut surfer_integration = SurferBlockParser::None;
    let mut surfer_start: Option<usize> = None;

    for (event, range) in Parser::new(content).into_offset_iter() {
        match event {
            // The Html event gets fired for html block start, end and their contents
            // So we need some internal state to keep track if we are inside of a <surfer> block or
            // not
            Event::Html(raw) => {
                let html_tag = raw.trim();
                // This does not handle nested <surfer> HTML tags, but just do not do that. Simple-as
                enum SurferHtmlState<'a> {
                    BeginBlock,
                    Item(&'a str),
                    EndBlock,
                }

                let html_state = match html_tag {
                    "<surfer>" => SurferHtmlState::BeginBlock,
                    "</surfer>" => SurferHtmlState::EndBlock,
                    s @ _ => SurferHtmlState::Item(s),
                };

                let current_state = match html_state {
                    SurferHtmlState::BeginBlock => {
                        surfer_start = Some(range.start);
                        Ok(SurferBlockParser::Builder(SurferBlockBuilder::new()))
                    }
                    SurferHtmlState::Item(item) => surfer_block(item, surfer_integration),
                    SurferHtmlState::EndBlock => match surfer_integration {
                        SurferBlockParser::None => {
                            Err(anyhow!("</surfer> tag found, but no starting <surfer> tag"))
                        }
                        SurferBlockParser::Builder(surfer_block_builder) => {
                            Ok(SurferBlockParser::Final(surfer_block_builder.try_into()?))
                        }
                        SurferBlockParser::Final(_) => Err(anyhow!(
                            "Double </surfer> tags found with no matching <surfer> tag"
                        )),
                    },
                };

                let line = content[..range.end.min(content.len())]
                    .bytes()
                    .filter(|byte| *byte == b'\n')
                    .count();

                surfer_integration =
                    current_state.map_err(|e| e.context(format!("at {source}:{line}:1",)))?;

                if let SurferBlockParser::Final(surfer_block) = surfer_integration {
                    let Some(start) = surfer_start else {
                        bail!("Got into the situation where final tag is encountered without a starting surfer tag, earlier safeguards apparently failed, report this as a bug please :)")
                    };

                    blocks.push(Block {
                        kind: BlockKind::Surfer(surfer_block),
                        index: blocks.len() + 1,
                        line: line,
                        start: start,
                        end: range.end,
                    });
                    surfer_integration = SurferBlockParser::None;
                }
            }
            Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(info))) => {
                active = Some((info.to_string(), range.start, String::new()));
            }
            Event::Text(text) => {
                if let Some((_, _, code)) = &mut active {
                    code.push_str(&text);
                }
            }
            Event::End(TagEnd::CodeBlock) => {
                let Some((info, start, code)) = active.take() else {
                    continue;
                };
                let line = content[..start.min(content.len())]
                    .bytes()
                    .filter(|byte| *byte == b'\n')
                    .count()
                    + 1;
                let Some(attrs) = attributes(&info).map_err(|error| {
                    anyhow!("invalid block attributes at {source}:{line}:1: {error}")
                })?
                else {
                    continue;
                };

                if attrs.hidden && attrs.group.is_none() {
                    bail!("hidden requires group=<identifier> at {source}:{line}:1");
                }
                if !attrs.yosys.is_empty() && !attrs.netlistsvg {
                    bail!("yosys=<commands> requires netlistsvg at {source}:{line}:1");
                }
                if attrs.netlistsvg && attrs.top_entity.is_none() {
                    bail!("netlistsvg requires topEntity=<binding> at {source}:{line}:1");
                }

                blocks.push(Block {
                    kind: BlockKind::Code(CodeBlock { attrs, code }),
                    index: blocks.len() + 1,
                    line,
                    start,
                    end: range.end,
                });
            }
            _ => {}
        }
    }

    Ok(blocks)
}

pub(crate) fn units(blocks: &[Block]) -> Vec<Vec<&Block>> {
    let mut units = Vec::<Vec<&Block>>::new();
    let mut groups = HashMap::<&str, usize>::new();
    for block in blocks {
        if let Some(group) = block.get_group() {
            let index = *groups.entry(group).or_insert_with(|| {
                units.push(Vec::new());
                units.len() - 1
            });
            units[index].push(block);
        } else {
            units.push(vec![block]);
        }
    }
    units
}

fn surfer_block(
    item: &str,
    surfer_integration: SurferBlockParser,
) -> anyhow::Result<SurferBlockParser> {
    surfer_integration.map_builder(|builder| {
        let (property, value) = match item.split_once("=").map(|(a, b)| (a.trim(), b.trim())) {
            Some((key, value)) => (SurferBlockProperty::try_from(key)?, value),
            None => bail!("Non-property being set in <surfer> block"),
        };

        macro_rules! parse_u32 {
            ($prop:literal, $value:expr) => {
                match $value.parse() {
                    Ok(n) => n,
                    Err(err) => {
                        bail!(concat!($prop, " property is not a u32: {}"), err);
                    }
                }
            };
        }

        Ok(match property {
            SurferBlockProperty::Group => builder.with_group(value.to_string()),
            SurferBlockProperty::CycleStart => {
                builder.with_cycle_start(parse_u32!("cycle_start", value))
            }
            SurferBlockProperty::CycleEnd => builder.with_cycle_end(parse_u32!("cycle_end", value)),
            SurferBlockProperty::Width => builder.with_width(parse_u32!("width", value)),
            SurferBlockProperty::Height => builder.with_height(parse_u32!("height", value)),
            SurferBlockProperty::Signals => {
                builder.with_signals(value.split(",").map(|s| s.trim().to_string()).collect())
            }
            SurferBlockProperty::SurferBatch => {
                builder.with_surfer_batch(value.split(",").map(|s| s.trim().to_string()).collect())
            }
        })
    })
}

fn attributes(info: &str) -> Result<Option<Attributes>> {
    let (language, trailing) = info.split_once(char::is_whitespace).unwrap_or((info, ""));
    let mut prefix = language.split(',');
    let language = prefix.next().unwrap_or_default();
    let mut values = prefix.collect::<Vec<_>>();

    if language != "clash" && !values.contains(&"clash") {
        return Ok(None);
    }
    if language != "haskell" && language != "clash" {
        bail!("expected `haskell`, found `{language}`");
    }
    values.retain(|value| *value != "clash");

    let mut attrs = Attributes::default();
    for value in values
        .into_iter()
        .map(str::to_string)
        .chain(shell_words::split(trailing).context("invalid quoted attribute")?)
    {
        if value == "netlistsvg" {
            if attrs.netlistsvg {
                bail!("netlistsvg was specified more than once");
            }
            attrs.netlistsvg = true;
        } else if value == "hidden" {
            if attrs.hidden {
                bail!("hidden was specified more than once");
            }
            attrs.hidden = true;
        } else if let Some(group) = value.strip_prefix("group=") {
            if group.is_empty() {
                bail!("group requires an identifier");
            }
            if attrs.group.replace(group.to_string()).is_some() {
                bail!("group was specified more than once");
            }
        } else if let Some(binding) = value.strip_prefix("topEntity=") {
            if binding.is_empty() {
                bail!("topEntity requires a binding name");
            }
            if attrs.top_entity.replace(binding.to_string()).is_some() {
                bail!("topEntity was specified more than once");
            }
        } else if let Some(commands) = value.strip_prefix("yosys=") {
            if !attrs.yosys.is_empty() {
                bail!("yosys was specified more than once");
            }
            attrs.yosys = commands
                .split(';')
                .map(str::trim)
                .filter(|command| !command.is_empty())
                .map(str::to_string)
                .collect();
            if attrs.yosys.is_empty() {
                bail!("yosys requires at least one command");
            }
        } else {
            bail!("unknown attribute `{value}`");
        }
    }

    Ok(Some(attrs))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_clash_blocks() {
        let content = r#"
```haskell
ignored
```

```haskell,clash
double x = x + x

>>> double 1
2
```

~~~haskell,clash topEntity=adder
adder x y = x + y
~~~
"#;
        let blocks: Vec<CodeBlock> = blocks(content, "chapter.md")
            .unwrap()
            .into_iter()
            .map(|block| match block.kind {
                BlockKind::Code(code_block) => code_block,
                BlockKind::Surfer(_surfer_block) => panic!("Not a code block!"),
            })
            .collect();
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].attrs.top_entity, None);
        assert_eq!(blocks[1].attrs.top_entity.as_deref(), Some("adder"));
    }
}
