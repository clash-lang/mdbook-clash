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

#[derive(Clone, Debug, Default)]
pub(crate) struct Attributes {
    pub group: Option<String>,
    pub top_entity: Option<String>,
    pub yosys: Vec<String>,
    pub netlistsvg: bool,
    pub hidden: bool,
}

pub(crate) fn blocks(content: &str, source: &str) -> Result<Vec<Block>> {
    let mut blocks = Vec::new();
    let mut active: Option<(String, usize, String)> = None;

    for (event, range) in Parser::new(content).into_offset_iter() {
        match event {
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
