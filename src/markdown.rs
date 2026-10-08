use anyhow::{anyhow, bail, Context, Result};
use pulldown_cmark::{CodeBlockKind, Event, Parser, Tag, TagEnd};
use std::collections::HashMap;

#[derive(Clone, Debug)]
pub(crate) struct Block {
    pub attrs: Attributes,
    pub code: String,
    pub index: usize,
    pub line: usize,
    pub start: usize,
    pub end: usize,
}

#[derive(Clone, Debug, Default, serde::Serialize)]
pub(crate) struct ShockwavesAttributes {
    pub start: u32,
    pub end: u32,
    pub signals: Vec<String>,
    pub resolution: (u32, u32),
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Attributes {
    pub group: Option<String>,
    pub top_entity: Option<String>,
    pub yosys: Vec<String>,
    pub shockwaves: Option<ShockwavesAttributes>,
    pub surfer_batch: Vec<String>,
    pub netlistsvg: bool,
    pub hidden: bool,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct SurferBlock {
    pub group: String,
    pub cycle_start: u32,
    pub cycle_end: u32,
    pub signals: Vec<String>,
    pub surfer_batch: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct SurferBlockBuilder {
    pub group: Option<String>,
    pub cycle_start: Option<u32>,
    pub cycle_end: Option<u32>,
    pub signals: Vec<String>,
    pub surfer_batch: Vec<String>,
}

impl SurferBlockBuilder {
    fn new() -> Self {
        Self::default()
    }

    fn with_group(mut self, group: String) -> Self {
        self.group = Some(group);
        self
    }

    fn with_cycle_end(mut self, cycle_end: u32) -> Self {
        self.cycle_end = Some(cycle_end);
        self
    }

    fn with_cycle_start(mut self, cycle_start: u32) -> Self {
        self.cycle_start = Some(cycle_start);
        self
    }

    fn with_signals(mut self, signals: Vec<String>) -> Self {
        self.signals = signals;
        self
    }

    fn with_surfer_batch(mut self, surfer_batch: Vec<String>) -> Self {
        self.surfer_batch = surfer_batch;
        self
    }
}

impl TryFrom<SurferBlockBuilder> for SurferBlock {
    type Error = &'static str;

    fn try_from(mut value: SurferBlockBuilder) -> std::prelude::v1::Result<Self, Self::Error> {
        let Some(group) = value.group.take() else {
            return Err("No group specified, do not know what code group to grab signals from");
        };
        let Some(cycle_start) = value.cycle_start.take() else {
            return Err(
                "No cycle_start specified, do not know which cycle to start the simulation from",
            );
        };
        let Some(cycle_end) = value.cycle_end.take() else {
            return Err("No cycle_end, do not know which cycle to end the simulation");
        };
        if value.signals.is_empty() {
            log::warn!(target: "surfer-integration", "No signals specified to monitor, an empty image will be generated")
        }
        Ok(SurferBlock {
            group,
            cycle_start,
            cycle_end,
            signals: value.signals,
            surfer_batch: value.surfer_batch,
        })
    }
}

pub(crate) fn blocks(content: &str, source: &str) -> Result<Vec<Block>> {
    let mut blocks = Vec::new();
    let mut active: Option<(String, usize, String)> = None;

    let mut surfer_integration = None;

    for (event, range) in Parser::new(content).into_offset_iter() {
        match event {
            // The Html event gets fired for html block start, end and their contents
            // So we need some internal state to keep track if we are inside of a <surfer> block or
            // not
            Event::Html(raw) => {
                let html = raw.trim();
                // This does not handle nested <surfer> HTML tags, but just do not do that. Simple-as
                enum SurferHtmlState<'a> {
                    BeginBlock,
                    Item(&'a str),
                    EndBlock,
                }

                let state = match html {
                    "<surfer>" => SurferHtmlState::BeginBlock,
                    "</surfer>" => SurferHtmlState::EndBlock,
                    s @ _ => SurferHtmlState::Item(s),
                };

                match state {
                    SurferHtmlState::BeginBlock => {
                        surfer_integration = Some(SurferBlockBuilder::new());
                    }
                    SurferHtmlState::Item(raw_item) => {
                        surfer_integration = surfer_integration.map(|builder| {
                            enum SurferBlockProperty {
                                Group(String),
                                CycleEnd(u32),
                                CycleStart(u32),
                                Signals(Vec<String>),
                                Batch(Vec<String>),
                            }

                            let property = match raw_item.split_once("=").map(|(a,b)| (a.trim(), b.trim())) {
                                Some(("group", value)) => {
                                    SurferBlockProperty::Group(value.trim().to_string())
                                }
                                Some(("cycle_start", value)) => {
                                    let n = match value.parse() {
                                        Ok(n) => n,
                                        Err(err) => {
                                            log::error!(target: "surfer-integration", "cycle_start property is not a u32: {err}");
                                            panic!("cycle_start property is not a u32: {err}");
                                        }
                                    };
                                    SurferBlockProperty::CycleStart(n)
                                }
                                Some(("cycle_end", value)) => {
                                    let n = match value.parse() {
                                        Ok(n) => n,
                                        Err(err) => {
                                            log::error!(target: "surfer-integration", "cycle_end property is not a u32: {err}");
                                            panic!("cycle_end property is not a u32: {err}");
                                        }
                                    };
                                    SurferBlockProperty::CycleEnd(n)
                                }
                                Some(("signals", value)) => SurferBlockProperty::Signals(
                                    value.split(",").map(|s| s.trim().to_string()).collect(),
                                ),
                                Some(("surfer_batch", value)) => SurferBlockProperty::Batch(
                                    value.split(",").map(|s| s.trim().to_string()).collect(),
                                ),
                                Some((key, value)) => {
                                    log::error!(target: "surfer-integration", "Unrecognized property '{key}' in <surfer> block (value: {value})");
                                    panic!("Unrecognized property '{key}' in <surfer> block (value: {value})");
                                },
                                _ => {
                                    log::error!(target: "surfer-integration", "Non-property being set in <surfer> block");
                                    panic!("Non-property being set in <surfer> block");
                                }
                            };

                            match property {
                                SurferBlockProperty::Group(group) => builder.with_group(group),
                                SurferBlockProperty::CycleEnd(end) => builder.with_cycle_end(end),
                                SurferBlockProperty::CycleStart(start) => {
                                    builder.with_cycle_start(start)
                                }
                                SurferBlockProperty::Signals(items) => builder.with_signals(items),
                                SurferBlockProperty::Batch(items) => builder.with_surfer_batch(items),
                            }
                        });
                    }
                    SurferHtmlState::EndBlock => {
                        let surfer: SurferBlock = match surfer_integration
                            .take()
                            .ok_or("</surfer> tag found, but no starting <surfer> tag")
                            .and_then(|builder| builder.try_into())
                        {
                            Ok(f) => f,
                            Err(e) => {
                                log::error!(target: "surfer-integration", "{e}");
                                panic!("{e}");
                            }
                        };
                        dbg!(surfer);
                    }
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
                    attrs,
                    code,
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
        if let Some(group) = block.attrs.group.as_deref() {
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
        let blocks = blocks(content, "chapter.md").unwrap();
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].attrs.top_entity, None);
        assert_eq!(blocks[1].attrs.top_entity.as_deref(), Some("adder"));
    }
}
