use crate::markdown::Block;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub(crate) struct Doctest<'a> {
    pub definitions: &'a str,
    pub transcript: &'a str,
    pub line_offset: usize,
}

pub(crate) fn doctest(code: &str) -> Doctest<'_> {
    let mut offset = 0;
    for (line_index, line) in code.split_inclusive('\n').enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with(">>>") || trimmed.starts_with("prop>") {
            return Doctest {
                definitions: code[..offset].trim_end_matches('\n'),
                transcript: &code[offset..],
                line_offset: line_index + 1,
            };
        }
        offset += line.len();
    }
    Doctest {
        definitions: code.trim_end_matches('\n'),
        transcript: "",
        line_offset: 0,
    }
}

pub(crate) fn suffix_shockwave_main(block: &Block) -> String {
    if let Some(shockwaves) = &block.attrs.shockwaves {
        // TODO: support explicit CRE signals
        let cycle_start = shockwaves.start;
        let cycle_end = shockwaves.end;
        let unwrapped_signals: String = shockwaves.signals.iter().map(|signal_name| format!("  let {signal_name}_unwrapped__ = exposeClockResetEnable {signal_name} systemClockGen systemResetGen enableGen\n")).collect();
        let bundle_with_comma: String = shockwaves.signals.iter().map(|signal_name| format!("Clash.Shockwaves.traceSignal \"{signal_name}\" {signal_name}_unwrapped__, ")).collect();
        let bundle = bundle_with_comma.trim_end_matches(", ");
        let bundle_names_with_comma: String = shockwaves
            .signals
            .iter()
            .map(|signal_name| format!("\"{signal_name}\", "))
            .collect();
        let bundle_names = bundle_names_with_comma.trim_end_matches(", ");
        format!(
            r#"
main :: IO ()
main = do
{unwrapped_signals}
  vcddata <- dumpVCD ({cycle_start}, {cycle_end}) (bundle ({bundle})) [ {bundle_names} ]
  case vcddata of
    Left msg -> error msg
    Right (vcd, meta) -> do
      writeFile     "waveform.vcd"  vcd
      writeFileJSON "waveform.json" meta
"#
        )
    } else {
        String::new()
    }
}

pub(crate) fn assemble(blocks: &[&Block]) -> String {
    let mut definitions = blocks
        .iter()
        .map(|block| doctest(&block.code).definitions)
        .collect::<Vec<_>>()
        .join("\n\n");
    let shockwave_block =blocks
        .iter()
        .find(|block| block.attrs.shockwaves.is_some())
        .map(|block| suffix_shockwave_main(block))
        .unwrap_or(String::new());
    definitions.push_str("\n\n");
    definitions.push_str(&shockwave_block);
    definitions.push_str("\n\n");
    definitions
}

pub(crate) fn module_name(source: &str) -> String {
    source
        .lines()
        .map(str::trim_start)
        .find_map(|line| {
            let declaration = line.strip_prefix("module ")?.trim_start();
            let name = declaration
                .chars()
                .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '_' || *ch == '.')
                .collect::<String>();
            (!name.is_empty()).then_some(name)
        })
        .unwrap_or_else(|| "Main".to_string())
}

pub(crate) fn module_path(directory: &Path, module: &str) -> PathBuf {
    let mut path = directory.to_path_buf();
    for component in module.split('.') {
        path.push(component);
    }
    path.set_extension("hs");
    path
}

pub(crate) fn listing(anchor: &str, group: &str, source: &str) -> String {
    let longest = source.chars().fold((0, 0), |(longest, current), ch| {
        let current = if ch == '`' { current + 1 } else { 0 };
        (longest.max(current), current)
    });
    let fence = "`".repeat(longest.0.max(2) + 1);
    let group = group
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;");
    format!(
        "\n<a id=\"{anchor}\"></a>\n\n### Group <code>{group}</code>\n\n{fence}haskell\n{source}\n{fence}\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_doctest_document_from_definitions() {
        let section =
            doctest("double x = x + x\n\n>>> :{\nlet value = double 21\n:}\n>>> value\n42\n");
        assert_eq!(section.definitions, "double x = x + x");
        assert_eq!(
            section.transcript,
            ">>> :{\nlet value = double 21\n:}\n>>> value\n42\n"
        );
        assert_eq!(section.line_offset, 3);
    }

    #[test]
    fn recognizes_property_documents() {
        let section = doctest("identity x = x\n\nprop> identity x == x\n");
        assert_eq!(section.definitions, "identity x = x");
        assert_eq!(section.transcript, "prop> identity x == x\n");
    }

    #[test]
    fn finds_module_name() {
        assert_eq!(
            module_name("{-# LANGUAGE TypeApplications #-}\nmodule Example.Counter where\n"),
            "Example.Counter"
        );
        assert_eq!(module_name("value = 1\n"), "Main");
    }
}
