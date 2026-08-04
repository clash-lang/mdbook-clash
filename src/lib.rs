use anyhow::Result;
use fs2::FileExt;
use mdbook_preprocessor::book::{Book, BookItem, Chapter};
use mdbook_preprocessor::errors::Error;
use mdbook_preprocessor::{Preprocessor, PreprocessorContext};
use pulldown_cmark::{CodeBlockKind, Event, Parser, Tag, TagEnd};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

const CACHE_SCHEMA: u32 = 1;
static RUN_ID: AtomicU64 = AtomicU64::new(0);

pub struct ClashPreprocessor;

#[derive(Clone, Debug)]
struct Config {
    work_dir: PathBuf,
    run_dir: PathBuf,
    keep_artifacts: bool,
    cache: bool,
    cache_key: String,
    clash_cmd: Vec<String>,
    clash_args: Vec<String>,
    test_exe_args: Vec<String>,
    yosys_cmd: Vec<String>,
    netlistsvg_cmd: Vec<String>,
}

#[derive(Clone, Debug)]
struct ClashBlock {
    attrs: BlockAttrs,
    code: String,
    block_index: usize,
    start_line: usize,
    start_offset: usize,
    end_offset: usize,
}

#[derive(Clone, Debug, Default)]
struct BlockAttrs {
    group: Option<String>,
    top_entity: Option<String>,
    yosys_commands: Vec<String>,
    netlistsvg: bool,
    hidden: bool,
}

#[derive(Clone, Debug)]
struct ProcessContext<'a> {
    ctx: &'a PreprocessorContext,
    config: Config,
}

impl Preprocessor for ClashPreprocessor {
    fn name(&self) -> &str {
        "mdbook-clash"
    }

    fn supports_renderer(&self, renderer: &str) -> Result<bool, anyhow::Error> {
        Ok(renderer == "html")
    }

    fn run(&self, ctx: &PreprocessorContext, mut book: Book) -> Result<Book, Error> {
        let config = Config::from_context(ctx)?;
        let process_context = ProcessContext { ctx, config };

        let result: Result<(), Error> = (|| {
            for item in book.items.iter_mut() {
                process_item(&process_context, item)?;
            }
            Ok(())
        })();
        let cleanup_result = cleanup_ephemeral_files(&process_context);

        result?;
        cleanup_result?;

        Ok(book)
    }
}

fn process_item(process_context: &ProcessContext, item: &mut BookItem) -> Result<(), Error> {
    match item {
        BookItem::Chapter(chapter) => process_chapter(process_context, chapter),
        _ => Ok(()),
    }
}

fn process_chapter(process_context: &ProcessContext, chapter: &mut Chapter) -> Result<(), Error> {
    let chapter_path = chapter
        .path
        .as_ref()
        .cloned()
        .unwrap_or_else(|| PathBuf::from("<unknown chapter>"));
    let chapter_path = resolve_chapter_path(process_context, &chapter_path);

    let blocks = find_clash_blocks(&chapter.content, &chapter_path)?;
    for block in &blocks {
        validate_block(&chapter_path, block)?;
    }

    let mut units: Vec<Vec<&ClashBlock>> = Vec::new();
    let mut grouped_units = HashMap::new();
    for block in &blocks {
        if let Some(group) = block.attrs.group.as_deref() {
            let unit_index = *grouped_units.entry(group).or_insert_with(|| {
                units.push(Vec::new());
                units.len() - 1
            });
            units[unit_index].push(block);
        } else {
            units.push(vec![block]);
        }
    }

    let mut edits = Vec::new();
    let mut listings = Vec::new();
    for unit in units {
        if unit.iter().any(|block| has_doctests(&block.code)) {
            test_blocks(process_context, &chapter_path, &unit)?;
        }
        let definitions = assembled_definitions(&unit);
        let listing_link = unit[0].attrs.group.as_deref().map(|group| {
            let anchor = format!("mdbook-clash-listing-{}", listings.len() + 1);
            listings.push(render_group_listing(&anchor, group, &definitions));
            format!("\n\n[View full listing](#{anchor})\n")
        });
        for block in unit {
            let addition = process_block(process_context, &chapter_path, block, &definitions)?;
            if block.attrs.hidden {
                edits.push((block.start_offset, block.end_offset, addition));
            } else {
                let replacement = format!("{}{addition}", listing_link.as_deref().unwrap_or(""));
                if !replacement.is_empty() {
                    edits.push((block.end_offset, block.end_offset, replacement));
                }
            }
        }
    }

    edits.sort_by_key(|(start, _, _)| *start);
    for (start, end, replacement) in edits.into_iter().rev() {
        chapter.content.replace_range(start..end, &replacement);
    }
    if !listings.is_empty() {
        chapter.content.push_str("\n\n## Full code listings\n");
        for listing in listings {
            chapter.content.push_str(&listing);
        }
    }

    for sub_item in chapter.sub_items.iter_mut() {
        process_item(process_context, sub_item)?;
    }

    Ok(())
}

fn resolve_chapter_path(process_context: &ProcessContext, chapter_path: &Path) -> PathBuf {
    if chapter_path.is_absolute() || chapter_path == Path::new("<unknown chapter>") {
        return chapter_path.to_path_buf();
    }

    let source_dir = &process_context.ctx.config.book.src;
    if chapter_path.starts_with(source_dir) {
        process_context.ctx.root.join(chapter_path)
    } else {
        process_context.ctx.root.join(source_dir).join(chapter_path)
    }
}

fn display_source_path(path: &Path) -> String {
    let terminal_dir = std::env::var_os("PWD")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| std::env::current_dir().ok());
    if let Some(terminal_dir) = terminal_dir {
        if let Ok(relative) = path.strip_prefix(terminal_dir) {
            if !relative.as_os_str().is_empty() {
                return relative.display().to_string();
            }
        }
    }
    path.display().to_string()
}

fn process_block(
    process_context: &ProcessContext,
    chapter_path: &Path,
    block: &ClashBlock,
    definitions: &str,
) -> Result<String, Error> {
    let Some(top_entity) = block.attrs.top_entity.as_deref() else {
        return Ok(String::new());
    };
    let module_name = source_module_name(definitions);
    let synthesis = synthesize_module(
        process_context,
        chapter_path,
        block,
        &module_name,
        top_entity,
        definitions,
    )?;
    if block.attrs.netlistsvg {
        netlistsvg_block(process_context, chapter_path, block, &synthesis)
    } else {
        Ok(String::new())
    }
}

fn validate_block(chapter_path: &Path, block: &ClashBlock) -> Result<(), Error> {
    if block.attrs.hidden && block.attrs.group.is_none() {
        return Err(Error::msg(format!(
            "mdbook-clash: hidden requires group=<identifier>\n\
             source: {}:{}:1\n\
             note: Hidden blocks must belong to a group so their source can be included elsewhere",
            display_source_path(chapter_path),
            block.start_line
        )));
    }

    if !block.attrs.yosys_commands.is_empty() && !block.attrs.netlistsvg {
        return Err(Error::msg(format!(
            "mdbook-clash: yosys=<commands> requires netlistsvg\n\
             source: {}:{}:1\n\
             note: Yosys commands are only used to generate netlist diagrams",
            display_source_path(chapter_path),
            block.start_line
        )));
    }

    if block.attrs.netlistsvg && block.attrs.top_entity.is_none() {
        return Err(Error::msg(format!(
            "mdbook-clash: netlistsvg requires topEntity=<binding>\n\
             source: {}:{}:1",
            display_source_path(chapter_path),
            block.start_line
        )));
    }

    Ok(())
}

#[derive(Debug)]
struct SynthesisResult {
    verilog_dir: PathBuf,
    top_component: String,
    cache_key: String,
    output_hash: String,
}

fn synthesize_module(
    process_context: &ProcessContext,
    chapter_path: &Path,
    block: &ClashBlock,
    module_name: &str,
    top_entity: &str,
    source: &str,
) -> Result<SynthesisResult, Error> {
    let cache_key = phase_cache_key(
        process_context,
        "synth",
        serde_json::json!({
            "source": source,
            "top_entity": top_entity,
            "command": process_context.config.clash_cmd,
            "command_fingerprint": command_fingerprint(&process_context.config.clash_cmd),
            "args": process_context.config.clash_args,
            "hdl": "verilog",
        }),
    )?;
    let artifact_dir = artifact_dir(process_context, "synth", &cache_key);
    let _lock = lock_artifact(&artifact_dir)?;
    let module_path = module_path_for(&artifact_dir.join("src"), module_name);
    let verilog_dir = artifact_dir.join("verilog");

    if cache_hit(process_context, &artifact_dir, "synth", &cache_key) {
        eprintln!(
            " INFO mdbook-clash: synthesis cache hit for {}:{}",
            display_source_path(chapter_path),
            block.start_line
        );
        let top_component = read_top_component(&verilog_dir)?;
        let output_hash = directory_hash(&verilog_dir)?;
        return Ok(SynthesisResult {
            verilog_dir,
            top_component,
            cache_key,
            output_hash,
        });
    }

    if artifact_dir.exists() {
        fs::remove_dir_all(&artifact_dir)
            .map_err(|err| Error::msg(format!("clearing artifact directory failed: {err}")))?;
    }
    fs::create_dir_all(module_path.parent().expect("module file has parent"))
        .map_err(|err| Error::msg(format!("creating source directory failed: {err}")))?;
    fs::create_dir_all(&verilog_dir)
        .map_err(|err| Error::msg(format!("creating Verilog directory failed: {err}")))?;
    fs::write(&module_path, source)
        .map_err(|err| Error::msg(format!("writing generated Clash module failed: {err}")))?;

    let mut args = process_context.config.clash_args.clone();
    args.push(module_path.display().to_string());
    args.push("--verilog".to_string());
    args.push("-main-is".to_string());
    args.push(top_entity.to_string());
    args.push("-outputdir".to_string());
    args.push(verilog_dir.display().to_string());

    eprintln!(
        " INFO mdbook-clash: synthesizing {}:{}",
        display_source_path(chapter_path),
        block.start_line
    );
    eprintln!(
        " INFO mdbook-clash: running {}",
        shell_join_all(&process_context.config.clash_cmd, &args)
    );

    let output =
        run_configured_command(&process_context.config.clash_cmd, &args).map_err(|err| {
            command_error(
                "synthesis",
                chapter_path,
                block,
                &module_path,
                &process_context.config.clash_cmd,
                &args,
                err,
            )
        })?;

    if !output.status.success() {
        return Err(command_failure(
            "synthesis",
            chapter_path,
            block,
            &module_path,
            &process_context.config.clash_cmd,
            &args,
            &output,
        ));
    }

    let top_component = read_top_component(&verilog_dir)?;
    let output_hash = directory_hash(&verilog_dir)?;
    write_cache_manifest(process_context, &artifact_dir, "synth", &cache_key)?;

    eprintln!(
        " INFO mdbook-clash: synthesized {}:{}",
        display_source_path(chapter_path),
        block.start_line
    );

    Ok(SynthesisResult {
        verilog_dir,
        top_component,
        cache_key,
        output_hash,
    })
}

fn netlistsvg_block(
    process_context: &ProcessContext,
    chapter_path: &Path,
    block: &ClashBlock,
    synthesis: &SynthesisResult,
) -> Result<String, Error> {
    let cache_key = phase_cache_key(
        process_context,
        "netlistsvg",
        serde_json::json!({
            "synthesis": synthesis.cache_key,
            "synthesis_output": synthesis.output_hash,
            "top_component": synthesis.top_component,
            "yosys_commands": block.attrs.yosys_commands,
            "yosys_command": process_context.config.yosys_cmd,
            "yosys_fingerprint": command_fingerprint(&process_context.config.yosys_cmd),
            "netlistsvg_command": process_context.config.netlistsvg_cmd,
            "netlistsvg_fingerprint": command_fingerprint(&process_context.config.netlistsvg_cmd),
        }),
    )?;
    let artifact_dir = artifact_dir(process_context, "netlistsvg", &cache_key);
    let _lock = lock_artifact(&artifact_dir)?;
    let script_path = artifact_dir.join("netlist.ys");
    let json_path = artifact_dir.join("netlist.json");
    let svg_path = artifact_dir.join("netlist.svg");

    if cache_hit(process_context, &artifact_dir, "netlistsvg", &cache_key) {
        eprintln!(
            " INFO mdbook-clash: netlistsvg cache hit for {}:{}",
            display_source_path(chapter_path),
            block.start_line
        );
        let svg = fs::read_to_string(&svg_path)
            .map_err(|err| Error::msg(format!("reading cached netlistsvg output failed: {err}")))?;
        return Ok(render_netlistsvg_markdown(&sanitize_svg_for_docs(
            process_context,
            &svg,
        )));
    }

    if artifact_dir.exists() {
        fs::remove_dir_all(&artifact_dir).map_err(|err| {
            Error::msg(format!(
                "clearing netlistsvg artifact directory failed: {err}"
            ))
        })?;
    }
    fs::create_dir_all(&artifact_dir)
        .map_err(|err| Error::msg(format!("creating netlistsvg directory failed: {err}")))?;

    let verilog_files = find_verilog_files(&synthesis.verilog_dir)?;
    if verilog_files.is_empty() {
        return Err(Error::msg(format!(
            "mdbook-clash: no Verilog files found for netlistsvg\n\
             source: {}:{}:1\n\
             verilog-dir: {}",
            display_source_path(chapter_path),
            block.start_line,
            synthesis.verilog_dir.display()
        )));
    }

    let script_lines = render_yosys_json_script(
        &block.attrs.yosys_commands,
        &verilog_files,
        &synthesis.top_component,
        &json_path,
    );
    let script = script_lines.join("\n") + "\n";
    fs::write(&script_path, &script)
        .map_err(|err| Error::msg(format!("writing netlistsvg Yosys script failed: {err}")))?;

    let yosys_args = vec!["-s".to_string(), script_path.display().to_string()];
    eprintln!(
        " INFO mdbook-clash: running Yosys JSON export for {}:{}",
        display_source_path(chapter_path),
        block.start_line
    );
    eprintln!(
        " INFO mdbook-clash: running {}",
        shell_join_all(&process_context.config.yosys_cmd, &yosys_args)
    );

    let yosys_output = run_configured_command(&process_context.config.yosys_cmd, &yosys_args)
        .map_err(|err| {
            command_error(
                "Yosys JSON export",
                chapter_path,
                block,
                &script_path,
                &process_context.config.yosys_cmd,
                &yosys_args,
                err,
            )
        })?;

    if !yosys_output.status.success() {
        return Err(command_failure(
            "Yosys JSON export",
            chapter_path,
            block,
            &script_path,
            &process_context.config.yosys_cmd,
            &yosys_args,
            &yosys_output,
        ));
    }

    let netlistsvg_args = vec![
        json_path.display().to_string(),
        "-o".to_string(),
        svg_path.display().to_string(),
    ];
    eprintln!(
        " INFO mdbook-clash: running netlistsvg for {}:{}",
        display_source_path(chapter_path),
        block.start_line
    );
    eprintln!(
        " INFO mdbook-clash: running {}",
        shell_join_all(&process_context.config.netlistsvg_cmd, &netlistsvg_args)
    );

    let netlistsvg_output =
        run_configured_command(&process_context.config.netlistsvg_cmd, &netlistsvg_args).map_err(
            |err| {
                command_error(
                    "netlistsvg",
                    chapter_path,
                    block,
                    &json_path,
                    &process_context.config.netlistsvg_cmd,
                    &netlistsvg_args,
                    err,
                )
            },
        )?;

    if !netlistsvg_output.status.success() {
        return Err(command_failure(
            "netlistsvg",
            chapter_path,
            block,
            &json_path,
            &process_context.config.netlistsvg_cmd,
            &netlistsvg_args,
            &netlistsvg_output,
        ));
    }

    let svg = fs::read_to_string(&svg_path)
        .map_err(|err| Error::msg(format!("reading netlistsvg output failed: {err}")))?;
    write_cache_manifest(process_context, &artifact_dir, "netlistsvg", &cache_key)?;

    Ok(render_netlistsvg_markdown(&sanitize_svg_for_docs(
        process_context,
        &svg,
    )))
}

fn assembled_definitions(blocks: &[&ClashBlock]) -> String {
    let source = blocks
        .iter()
        .map(|block| strip_doctests(&block.code))
        .map(|source| source.trim_end_matches('\n').to_string())
        .collect::<Vec<_>>()
        .join("\n\n");
    if blocks
        .first()
        .and_then(|block| block.attrs.group.as_ref())
        .is_some()
    {
        hoist_imports(&source)
    } else {
        source
    }
}

fn hoist_imports(source: &str) -> String {
    let imports = source
        .lines()
        .filter(|line| line.starts_with("import "))
        .collect::<Vec<_>>();
    if imports.is_empty() {
        return source.to_string();
    }

    let mut output = Vec::new();
    let mut emitted_imports = false;
    for line in source.lines() {
        if line.starts_with("import ") {
            if !emitted_imports {
                output.extend(imports.iter().copied());
                emitted_imports = true;
            }
        } else {
            output.push(line);
        }
    }
    output.join("\n")
}

fn render_group_listing(anchor: &str, group: &str, source: &str) -> String {
    let fence = "`".repeat(longest_run(source, '`').max(2) + 1);
    format!(
        "\n<a id=\"{anchor}\"></a>\n\n### Group <code>{}</code>\n\n{fence}haskell\n{source}\n{fence}\n",
        escape_html(group)
    )
}

fn longest_run(value: &str, needle: char) -> usize {
    value
        .chars()
        .fold((0, 0), |(longest, current), character| {
            let current = if character == needle { current + 1 } else { 0 };
            (longest.max(current), current)
        })
        .0
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn test_blocks(
    process_context: &ProcessContext,
    chapter_path: &Path,
    blocks: &[&ClashBlock],
) -> Result<(), Error> {
    let first_block = blocks.first().expect("test unit is not empty");
    let test_blocks = blocks
        .iter()
        .copied()
        .filter(|block| has_doctests(&block.code))
        .collect::<Vec<_>>();
    let snippet_source = assembled_definitions(blocks);
    let snippet_module = source_module_name(&snippet_source);
    if snippet_module == "MdbookClashRunner" {
        return Err(Error::msg(format!(
            "mdbook-clash: source module name `MdbookClashRunner` is reserved\n\
             source: {}:{}:1",
            display_source_path(chapter_path),
            first_block.start_line
        )));
    }
    let runner_source = render_test_runner(blocks, &snippet_module);
    let cache_key = phase_cache_key(
        process_context,
        "test",
        serde_json::json!({
            "snippet_source": snippet_source,
            "snippet_module": snippet_module,
            "runner_source": runner_source,
            "compile_command": process_context.config.clash_cmd,
            "compiler_fingerprint": command_fingerprint(&process_context.config.clash_cmd),
            "compile_args": process_context.config.clash_args,
            "run_args": process_context.config.test_exe_args,
        }),
    )?;
    let artifact_dir = artifact_dir(process_context, "test", &cache_key);
    let _lock = lock_artifact(&artifact_dir)?;
    let source_dir = artifact_dir.join("src");
    let snippet_path = module_path_for(&source_dir, &snippet_module);
    let runner_path = module_path_for(&source_dir, "MdbookClashRunner");
    let exe_path = artifact_dir.join(executable_name("mdbook-clash-test"));

    if cache_hit(process_context, &artifact_dir, "test", &cache_key) {
        eprintln!(
            " INFO mdbook-clash: simulation cache hit for {}",
            display_source_path(chapter_path)
        );
        return Ok(());
    }

    if artifact_dir.exists() {
        fs::remove_dir_all(&artifact_dir)
            .map_err(|err| Error::msg(format!("clearing artifact directory failed: {err}")))?;
    }
    fs::create_dir_all(snippet_path.parent().expect("snippet file has a parent"))
        .map_err(|err| Error::msg(format!("creating test artifact directory failed: {err}")))?;
    fs::write(&snippet_path, snippet_source)
        .map_err(|err| Error::msg(format!("writing combined source module failed: {err}")))?;
    fs::write(&runner_path, runner_source)
        .map_err(|err| Error::msg(format!("writing generated test runner failed: {err}")))?;

    let mut compile_args = process_context.config.clash_args.clone();
    compile_args.push(runner_path.display().to_string());
    compile_args.push(format!("-i{}", source_dir.display()));
    compile_args.push("-main-is".to_string());
    compile_args.push("MdbookClashRunner.main".to_string());
    compile_args.push("-o".to_string());
    compile_args.push(exe_path.display().to_string());

    eprintln!(
        " INFO mdbook-clash: compiling simulation for {}",
        display_source_path(chapter_path)
    );
    eprintln!(
        " INFO mdbook-clash: compiling {}",
        shell_join_all(&process_context.config.clash_cmd, &compile_args)
    );

    let compile_output = run_configured_command(&process_context.config.clash_cmd, &compile_args)
        .map_err(|err| {
        command_error(
            "simulation compile",
            chapter_path,
            first_block,
            &runner_path,
            &process_context.config.clash_cmd,
            &compile_args,
            err,
        )
    })?;

    if !compile_output.status.success() {
        return Err(command_failure(
            "simulation compile",
            chapter_path,
            first_block,
            &runner_path,
            &process_context.config.clash_cmd,
            &compile_args,
            &compile_output,
        ));
    }

    for block in test_blocks {
        let run_args = process_context.config.test_exe_args.clone();
        eprintln!(
            " INFO mdbook-clash: simulating {}:{}",
            display_source_path(chapter_path),
            block.start_line
        );
        let run_output = run_configured_command_with_env(
            &[exe_path.display().to_string()],
            &run_args,
            "MDBOOK_CLASH_BLOCK",
            &block.block_index.to_string(),
        )
        .map_err(|err| {
            command_error(
                "simulation",
                chapter_path,
                block,
                &exe_path,
                &[exe_path.display().to_string()],
                &run_args,
                err,
            )
        })?;

        if !run_output.status.success() {
            return Err(command_failure(
                "simulation",
                chapter_path,
                block,
                &exe_path,
                &[exe_path.display().to_string()],
                &run_args,
                &run_output,
            ));
        }
    }

    write_cache_manifest(process_context, &artifact_dir, "test", &cache_key)?;

    eprintln!(
        " INFO mdbook-clash: simulated {}",
        display_source_path(chapter_path)
    );

    Ok(())
}

fn find_clash_blocks(content: &str, chapter_path: &Path) -> Result<Vec<ClashBlock>, Error> {
    let mut blocks = Vec::new();
    let mut block_index = 0usize;
    let mut active: Option<(String, usize, String)> = None;

    for (event, range) in Parser::new(content).into_offset_iter() {
        match event {
            Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(info))) => {
                active = Some((info.to_string(), range.start, String::new()));
            }
            Event::End(TagEnd::CodeBlock) => {
                let Some((info, start_offset, code)) = active.take() else {
                    continue;
                };
                let start_line = line_number_at_offset(content, start_offset);
                let Some(attrs) = parse_block_info(&info).map_err(|err| {
                    Error::msg(format!(
                        "mdbook-clash: invalid fenced block attributes: {err}\n\
                         source: {}:{start_line}:1",
                        display_source_path(chapter_path)
                    ))
                })?
                else {
                    continue;
                };
                block_index += 1;
                blocks.push(ClashBlock {
                    attrs,
                    code,
                    block_index,
                    start_line,
                    start_offset,
                    end_offset: range.end,
                });
            }
            Event::Text(text) => {
                let Some((_, _, code)) = &mut active else {
                    continue;
                };
                code.push_str(&text);
            }
            _ => {}
        }
    }

    Ok(blocks)
}

fn find_verilog_files(dir: &Path) -> Result<Vec<PathBuf>, Error> {
    Ok(files_below(dir)?
        .into_iter()
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("v"))
        .collect())
}

fn render_yosys_json_script(
    user_commands: &[String],
    verilog_files: &[PathBuf],
    top_component: &str,
    json_path: &Path,
) -> Vec<String> {
    let verilog = verilog_files
        .iter()
        .map(|path| yosys_quote(&path.display().to_string()))
        .collect::<Vec<_>>()
        .join(" ");
    let mut commands = vec![
        format!("read_verilog {verilog}"),
        format!("hierarchy -top {top_component}"),
    ];
    commands.extend(if user_commands.is_empty() {
        vec!["proc".to_string(), "opt".to_string()]
    } else {
        user_commands.to_vec()
    });
    commands.push("clean -purge".to_string());
    commands.push(format!(
        "write_json {}",
        yosys_quote(&json_path.display().to_string())
    ));
    commands
}

fn yosys_quote(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

fn render_netlistsvg_markdown(svg: &str) -> String {
    format!(
        r#"

#### Netlist

<div class="mdbook-clash-netlist" style="background: white; padding: 1rem; overflow-x: auto;">
{svg}
</div>
"#
    )
}

fn sanitize_svg_for_docs(process_context: &ProcessContext, svg: &str) -> String {
    let work_dir = process_context.config.work_dir.display().to_string();
    svg.replace(&work_dir, "mdbook-clash-work")
}

fn parse_block_info(info: &str) -> Result<Option<BlockAttrs>, String> {
    let (language, trailing_attributes) =
        info.split_once(char::is_whitespace).unwrap_or((info, ""));
    let mut prefix = language.split(',');
    let language = prefix.next().unwrap_or_default();
    let mut attributes = prefix.collect::<Vec<_>>();
    if language != "clash" && !attributes.contains(&"clash") {
        return Ok(None);
    }
    if language != "haskell" && language != "clash" {
        return Err(format!("expected `haskell`, found `{language}`"));
    }
    attributes.retain(|attribute| *attribute != "clash");
    let parsed_attributes =
        shell_words::split(trailing_attributes).map_err(|err| err.to_string())?;

    let mut attrs = BlockAttrs::default();
    for attr in attributes
        .into_iter()
        .map(str::to_string)
        .chain(parsed_attributes)
    {
        if attr == "netlistsvg" {
            attrs.netlistsvg = true;
        } else if attr == "hidden" {
            if attrs.hidden {
                return Err("hidden was specified more than once".to_string());
            }
            attrs.hidden = true;
        } else if let Some(value) = attr.strip_prefix("group=") {
            if value.is_empty() {
                return Err("group requires an identifier".to_string());
            }
            if attrs.group.replace(value.to_string()).is_some() {
                return Err("group was specified more than once".to_string());
            }
        } else if let Some(value) = attr.strip_prefix("topEntity=") {
            if value.is_empty() {
                return Err("topEntity requires a binding name".to_string());
            }
            if attrs.top_entity.replace(value.to_string()).is_some() {
                return Err("topEntity was specified more than once".to_string());
            }
        } else if let Some(value) = attr.strip_prefix("yosys=") {
            if !attrs.yosys_commands.is_empty() {
                return Err("yosys was specified more than once".to_string());
            }
            attrs.yosys_commands = split_yosys_commands(value);
            if attrs.yosys_commands.is_empty() {
                return Err("yosys requires at least one command".to_string());
            }
        } else {
            return Err(format!("unknown attribute `{attr}`"));
        }
    }

    Ok(Some(attrs))
}

fn split_yosys_commands(value: &str) -> Vec<String> {
    value
        .split(';')
        .map(str::trim)
        .filter(|command| !command.is_empty())
        .map(str::to_string)
        .collect()
}

fn line_number_at_offset(content: &str, offset: usize) -> usize {
    content[..offset.min(content.len())]
        .bytes()
        .filter(|byte| *byte == b'\n')
        .count()
        + 1
}

fn module_path_for(src_dir: &Path, module_name: &str) -> PathBuf {
    let mut path = src_dir.to_path_buf();
    for component in module_name.split('.') {
        path.push(component);
    }
    path.set_extension("hs");
    path
}

fn source_module_name(source: &str) -> String {
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

fn phase_cache_key(
    process_context: &ProcessContext,
    phase: &str,
    inputs: serde_json::Value,
) -> Result<String, Error> {
    let implementation_hash = implementation_hash()?;
    let data = serde_json::to_vec(&serde_json::json!({
        "schema": CACHE_SCHEMA,
        "version": env!("CARGO_PKG_VERSION"),
        "implementation": implementation_hash,
        "user_key": process_context.config.cache_key,
        "phase": phase,
        "inputs": inputs,
    }))
    .map_err(|err| Error::msg(format!("serializing cache key failed: {err}")))?;
    Ok(blake3::hash(&data).to_hex().to_string())
}

fn artifact_dir(process_context: &ProcessContext, phase: &str, cache_key: &str) -> PathBuf {
    let root = if process_context.config.cache {
        process_context
            .config
            .work_dir
            .join(format!("cache-v{CACHE_SCHEMA}"))
    } else {
        process_context.config.run_dir.clone()
    };
    root.join(phase).join(cache_key)
}

fn lock_artifact(artifact_dir: &Path) -> Result<fs::File, Error> {
    let parent = artifact_dir
        .parent()
        .expect("artifact directory has a parent");
    fs::create_dir_all(parent)
        .map_err(|err| Error::msg(format!("creating cache directory failed: {err}")))?;
    let lock = fs::File::create(artifact_dir.with_extension("lock"))
        .map_err(|err| Error::msg(format!("opening cache lock failed: {err}")))?;
    lock.lock_exclusive()
        .map_err(|err| Error::msg(format!("locking cache entry failed: {err}")))?;
    Ok(lock)
}

#[derive(Debug, Eq, PartialEq, Serialize, Deserialize)]
struct CacheManifest {
    schema: u32,
    version: String,
    phase: String,
    key: String,
    files: Vec<CachedFile>,
}

#[derive(Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
struct CachedFile {
    path: String,
    hash: String,
}

fn cache_hit(
    process_context: &ProcessContext,
    artifact_dir: &Path,
    phase: &str,
    key: &str,
) -> bool {
    if !process_context.config.cache {
        return false;
    }

    let marker = artifact_dir.join("cache.json");
    let Ok(contents) = fs::read(&marker) else {
        return false;
    };
    let Ok(manifest) = serde_json::from_slice::<CacheManifest>(&contents) else {
        return false;
    };
    let Ok(files) = cache_files(artifact_dir) else {
        return false;
    };

    manifest
        == CacheManifest {
            schema: CACHE_SCHEMA,
            version: env!("CARGO_PKG_VERSION").to_string(),
            phase: phase.to_string(),
            key: key.to_string(),
            files,
        }
}

fn write_cache_manifest(
    process_context: &ProcessContext,
    artifact_dir: &Path,
    phase: &str,
    key: &str,
) -> Result<(), Error> {
    if !process_context.config.cache {
        return Ok(());
    }

    let manifest = CacheManifest {
        schema: CACHE_SCHEMA,
        version: env!("CARGO_PKG_VERSION").to_string(),
        phase: phase.to_string(),
        key: key.to_string(),
        files: cache_files(artifact_dir)?,
    };
    let contents = serde_json::to_vec_pretty(&manifest)
        .map_err(|err| Error::msg(format!("serializing cache manifest failed: {err}")))?;
    let marker = artifact_dir.join("cache.json");
    let temporary = artifact_dir.join("cache.json.tmp");
    fs::write(&temporary, contents)
        .and_then(|()| fs::rename(&temporary, &marker))
        .map_err(|err| Error::msg(format!("writing cache manifest failed: {err}")))
}

fn cache_files(root: &Path) -> Result<Vec<CachedFile>, Error> {
    let mut files = files_below(root)?
        .into_iter()
        .filter(|path| path.file_name().and_then(|name| name.to_str()) != Some("cache.json"))
        .map(|path| {
            let contents = fs::read(&path)
                .map_err(|err| Error::msg(format!("reading cached output failed: {err}")))?;
            Ok(CachedFile {
                path: path
                    .strip_prefix(root)
                    .expect("cache output is below its root")
                    .to_string_lossy()
                    .into_owned(),
                hash: blake3::hash(&contents).to_hex().to_string(),
            })
        })
        .collect::<Result<Vec<_>, Error>>()?;
    files.sort();
    Ok(files)
}

fn directory_hash(root: &Path) -> Result<String, Error> {
    let files = cache_files(root)?;
    let contents = serde_json::to_vec(&files)
        .map_err(|err| Error::msg(format!("serializing artifact hashes failed: {err}")))?;
    Ok(blake3::hash(&contents).to_hex().to_string())
}

fn read_top_component(verilog_dir: &Path) -> Result<String, Error> {
    let manifests = files_below(verilog_dir)?
        .into_iter()
        .filter(|path| {
            path.file_name().and_then(|name| name.to_str()) == Some("clash-manifest.json")
        })
        .collect::<Vec<_>>();
    if manifests.len() != 1 {
        return Err(Error::msg(format!(
            "expected one Clash manifest below {}, found {}",
            verilog_dir.display(),
            manifests.len()
        )));
    }
    let contents = fs::read(&manifests[0])
        .map_err(|err| Error::msg(format!("reading Clash manifest failed: {err}")))?;
    let manifest: serde_json::Value = serde_json::from_slice(&contents)
        .map_err(|err| Error::msg(format!("parsing Clash manifest failed: {err}")))?;
    manifest["top_component"]["name"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| Error::msg("Clash manifest has no top component name"))
}

fn files_below(root: &Path) -> Result<Vec<PathBuf>, Error> {
    let mut directories = vec![root.to_path_buf()];
    let mut files = Vec::new();
    while let Some(directory) = directories.pop() {
        for entry in fs::read_dir(&directory)
            .map_err(|err| Error::msg(format!("reading artifact directory failed: {err}")))?
        {
            let path = entry
                .map_err(|err| Error::msg(format!("reading artifact entry failed: {err}")))?
                .path();
            if path.is_dir() {
                directories.push(path);
            } else {
                files.push(path);
            }
        }
    }
    files.sort();
    Ok(files)
}

fn executable_name(base: &str) -> String {
    if cfg!(windows) {
        format!("{base}.exe")
    } else {
        base.to_string()
    }
}

fn run_configured_command(cmd: &[String], args: &[String]) -> std::io::Result<Output> {
    configured_command(cmd, args).output()
}

fn run_configured_command_with_env(
    cmd: &[String],
    args: &[String],
    key: &str,
    value: &str,
) -> std::io::Result<Output> {
    let mut command = configured_command(cmd, args);
    command.env(key, value);
    command.output()
}

fn configured_command(cmd: &[String], args: &[String]) -> Command {
    let (program, prefix_args) = cmd
        .split_first()
        .expect("configured commands are validated as non-empty");
    let mut command = Command::new(program);
    command.args(prefix_args).args(args);
    command
}

fn command_error(
    mode: &str,
    chapter_path: &Path,
    block: &ClashBlock,
    generated_path: &Path,
    cmd: &[String],
    args: &[String],
    err: std::io::Error,
) -> Error {
    let hint = if err.kind() == std::io::ErrorKind::NotFound && mode == "netlistsvg" {
        "\n\
         hint: `netlistsvg` was requested by this block but was not found on PATH. \
         Run the book inside `nix develop`, install netlistsvg, \
         or set `netlistsvg-cmd` in book.toml."
    } else if err.kind() == std::io::ErrorKind::NotFound {
        "\n\
         hint: command executable was not found on PATH. Configure the command in book.toml \
         or run mdbook from an environment that provides it."
    } else {
        ""
    };

    Error::msg(format!(
        "mdbook-clash: failed to start {mode} command\n\
         source: {}:{}:1\n\
         generated: {}\n\
         command: {}\n\
         error: {err}{hint}",
        display_source_path(chapter_path),
        block.start_line,
        generated_path.display(),
        shell_join_all(cmd, args)
    ))
}

fn command_failure(
    mode: &str,
    chapter_path: &Path,
    block: &ClashBlock,
    generated_path: &Path,
    cmd: &[String],
    args: &[String],
    output: &Output,
) -> Error {
    let execution_details = if mode == "simulation" {
        String::new()
    } else {
        format!(
            "generated: {}\ncommand: {}\n",
            generated_path.display(),
            shell_join_all(cmd, args)
        )
    };
    Error::msg(format!(
        "mdbook-clash: {mode} failed\n\
         source: {}:{}:1\n\
         {execution_details}\
         status: {}\n\
         stdout:\n{}\n\
         stderr:\n{}",
        display_source_path(chapter_path),
        block.start_line,
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    ))
}

fn render_test_runner(blocks: &[&ClashBlock], snippet_module: &str) -> String {
    let mut module = String::new();
    module.push_str("module MdbookClashRunner where\n\n");
    module.push_str("import Clash.Prelude\n");
    module.push_str("import qualified Prelude as P\n");
    module.push_str("import System.Environment (lookupEnv)\n");
    module.push_str("import System.Exit (exitFailure)\n\n");
    module.push_str("import ");
    module.push_str(snippet_module);
    module.push_str("\n\n");
    module.push_str("__mdbookClashAssertEqual :: (Eq a, Show a) => P.String -> a -> a -> IO ()\n");
    module.push_str("__mdbookClashAssertEqual name expected actual =\n");
    module.push_str("  if expected == actual\n");
    module.push_str("    then P.pure ()\n");
    module.push_str("    else do\n");
    module.push_str("      P.putStrLn (\"FAILED: \" P.++ name)\n");
    module.push_str("      P.putStrLn (\"expected: \" P.++ P.show expected)\n");
    module.push_str("      P.putStrLn (\"actual:   \" P.++ P.show actual)\n");
    module.push_str("      exitFailure\n\n");
    module.push_str("main :: IO ()\n");
    module.push_str("main = do\n");
    module.push_str("  selector <- lookupEnv \"MDBOOK_CLASH_BLOCK\"\n");
    module.push_str("  case selector of\n");
    for block in blocks {
        let parsed = parse_doctest_lines(&block.code);
        if parsed.assertions.is_empty() {
            continue;
        }
        module.push_str(&format!("    P.Just \"{}\" -> do\n", block.block_index));
        for line in &parsed.assertions {
            module.push_str("      ");
            module.push_str(line);
            module.push('\n');
        }
    }
    module.push_str("    _ -> do\n");
    module.push_str("      P.putStrLn \"mdbook-clash: missing or invalid block selector\"\n");
    module.push_str("      exitFailure\n");

    module
}

#[derive(Debug)]
struct ParsedTest {
    definitions: String,
    assertions: Vec<String>,
}

fn parse_doctest_lines(code: &str) -> ParsedTest {
    let mut definitions = String::new();
    let mut assertions = Vec::new();
    let lines: Vec<&str> = code.lines().collect();
    let mut index = 0usize;
    let mut doctest_index = 0usize;

    while index < lines.len() {
        let line = lines[index];
        let trimmed = line.trim_start();

        if let Some(expr) = trimmed.strip_prefix(">>>") {
            doctest_index += 1;
            let expr = expr.trim();
            let expected = lines.get(index + 1).map(|line| line.trim()).unwrap_or("");
            assertions.push(format!(
                "__mdbookClashAssertEqual \"doctest {doctest_index}\" ({expected}) ({expr})"
            ));
            index += 2;
            continue;
        }

        definitions.push_str(line);
        definitions.push('\n');
        index += 1;
    }

    ParsedTest {
        definitions,
        assertions,
    }
}

fn has_doctests(code: &str) -> bool {
    code.lines()
        .any(|line| line.trim_start().starts_with(">>>"))
}

fn strip_doctests(code: &str) -> String {
    parse_doctest_lines(code).definitions
}

fn cleanup_ephemeral_files(process_context: &ProcessContext) -> Result<(), Error> {
    if process_context.config.keep_artifacts || process_context.config.cache {
        return Ok(());
    }

    let run_dir = &process_context.config.run_dir;
    if run_dir.exists() {
        fs::remove_dir_all(run_dir)
            .map_err(|err| Error::msg(format!("cleaning run artifact directory failed: {err}")))?;
    }

    Ok(())
}

impl Config {
    fn from_context(ctx: &PreprocessorContext) -> Result<Self, Error> {
        let work_dir = work_dir(ctx)?;
        let clash_cmd = get_command(ctx, "clash-cmd", &["clash"])?;
        let yosys_cmd = get_command(ctx, "yosys-cmd", &["yosys"])?;
        let netlistsvg_cmd = get_command(ctx, "netlistsvg-cmd", &["netlistsvg"])?;

        Ok(Self {
            run_dir: work_dir.join("runs").join(format!(
                "{}-{}",
                std::process::id(),
                RUN_ID.fetch_add(1, Ordering::Relaxed)
            )),
            work_dir,
            keep_artifacts: get_bool(ctx, "keep-artifacts", false)?,
            cache: get_bool(ctx, "cache", true)?,
            cache_key: get_string(ctx, "cache-key", "")?,
            clash_cmd,
            clash_args: get_string_vec(ctx, "clash-args", &[])?,
            test_exe_args: get_string_vec(ctx, "test-exe-args", &[])?,
            yosys_cmd,
            netlistsvg_cmd,
        })
    }
}

fn work_dir(ctx: &PreprocessorContext) -> Result<PathBuf, Error> {
    let configured = get_string(ctx, "work-dir", "mdbook-clash-work")?;
    let path = Path::new(&configured);
    if configured.is_empty()
        || path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir
                    | std::path::Component::RootDir
                    | std::path::Component::Prefix(_)
            )
        })
        || !path
            .components()
            .any(|component| matches!(component, std::path::Component::Normal(_)))
    {
        return Err(Error::msg(
            "`preprocessor.clash.work-dir` must be a non-empty relative path",
        ));
    }
    Ok(ctx.root.join(path))
}

fn get_string(ctx: &PreprocessorContext, key: &str, default: &str) -> Result<String, Error> {
    let config_key = format!("preprocessor.clash.{key}");
    match ctx.config.get::<String>(&config_key) {
        Ok(Some(value)) => Ok(value),
        Ok(None) => Ok(default.to_string()),
        Err(err) => Err(invalid_config(&config_key, err)),
    }
}

fn get_bool(ctx: &PreprocessorContext, key: &str, default: bool) -> Result<bool, Error> {
    let config_key = format!("preprocessor.clash.{key}");
    match ctx.config.get::<bool>(&config_key) {
        Ok(Some(value)) => Ok(value),
        Ok(None) => Ok(default),
        Err(err) => Err(invalid_config(&config_key, err)),
    }
}

fn get_string_vec(
    ctx: &PreprocessorContext,
    key: &str,
    default: &[&str],
) -> Result<Vec<String>, Error> {
    let config_key = format!("preprocessor.clash.{key}");
    match ctx.config.get::<Vec<String>>(&config_key) {
        Ok(Some(value)) => Ok(value),
        Ok(None) => Ok(default.iter().map(|value| value.to_string()).collect()),
        Err(err) => Err(invalid_config(&config_key, err)),
    }
}

fn get_command(
    ctx: &PreprocessorContext,
    key: &str,
    default: &[&str],
) -> Result<Vec<String>, Error> {
    let config_key = format!("preprocessor.clash.{key}");
    let command = get_string_vec(ctx, key, default)?;
    if command.is_empty() {
        Err(Error::msg(format!("`{config_key}` must not be empty")))
    } else {
        Ok(command)
    }
}

fn invalid_config(config_key: &str, err: impl std::fmt::Display) -> Error {
    Error::msg(format!("invalid `{config_key}`: {err}"))
}

fn implementation_hash() -> Result<String, Error> {
    let executable = std::env::current_exe()
        .map_err(|err| Error::msg(format!("locating mdbook-clash executable failed: {err}")))?;
    let contents = fs::read(executable)
        .map_err(|err| Error::msg(format!("reading mdbook-clash executable failed: {err}")))?;
    Ok(blake3::hash(&contents).to_hex().to_string())
}

fn command_fingerprint(command: &[String]) -> String {
    let Some(program) = command.first() else {
        return "missing".to_string();
    };
    let program = Path::new(program);
    let executable = if program.components().count() > 1 {
        program.to_path_buf()
    } else {
        std::env::var_os("PATH")
            .and_then(|path| {
                std::env::split_paths(&path)
                    .map(|directory| directory.join(program))
                    .find(|candidate| candidate.is_file())
            })
            .unwrap_or_else(|| program.to_path_buf())
    };

    fs::read(executable)
        .map(|contents| blake3::hash(&contents).to_hex().to_string())
        .unwrap_or_else(|_| "unavailable".to_string())
}

fn shell_join(args: &[String]) -> String {
    args.iter()
        .map(|arg| shell_words::quote(arg).into_owned())
        .collect::<Vec<_>>()
        .join(" ")
}

fn shell_join_all(cmd: &[String], args: &[String]) -> String {
    let mut all = cmd.to_vec();
    all.extend_from_slice(args);
    shell_join(&all)
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

        let blocks =
            find_clash_blocks(content, Path::new("chapter.md")).expect("valid block attributes");
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].attrs.top_entity, None);
        assert_eq!(blocks[1].attrs.top_entity.as_deref(), Some("adder"));
    }

    #[test]
    fn renders_doctest_assertion() {
        let block = ClashBlock {
            attrs: BlockAttrs::default(),
            code: r#"
double x = x + x

>>> double 21
42
"#
            .to_string(),
            block_index: 1,
            start_line: 1,
            start_offset: 0,
            end_offset: 0,
        };
        let runner = render_test_runner(&[&block], "Example");
        assert!(runner.contains("module MdbookClashRunner where"));
        assert!(runner.contains("import Example"));
        assert!(!runner.contains("double x = x + x"));
        assert!(runner.contains("__mdbookClashAssertEqual \"doctest 1\" (42) (double 21)"));
    }

    #[test]
    fn finds_user_source_module_name() {
        assert_eq!(
            source_module_name("{-# LANGUAGE TypeApplications #-}\nmodule Example.Counter where\n"),
            "Example.Counter"
        );
        assert_eq!(source_module_name("value = 1\n"), "Main");
    }

    #[test]
    fn collects_group_imports_at_the_first_import() {
        let source = "module Example where\n\nimport Clash.Prelude\n\nfirst = 1\n\nimport Data.Proxy (Proxy(..))\n\nsecond = Proxy @Int";
        assert_eq!(
            hoist_imports(source),
            "module Example where\n\nimport Clash.Prelude\nimport Data.Proxy (Proxy(..))\n\nfirst = 1\n\n\nsecond = Proxy @Int"
        );
    }

    #[test]
    fn renders_yosys_json_export_script() {
        let files = vec![PathBuf::from("/tmp/topEntity.v")];
        let user_commands = vec!["proc".to_string(), "opt".to_string(), "techmap".to_string()];
        let script = render_yosys_json_script(
            &user_commands,
            &files,
            "adder",
            Path::new("/tmp/netlist.json"),
        );
        assert!(script[0].starts_with("read_verilog "));
        assert!(script.contains(&"hierarchy -top adder".to_string()));
        assert!(script.contains(&"proc".to_string()));
        assert!(script.contains(&"opt".to_string()));
        assert!(script.contains(&"techmap".to_string()));
        assert!(script.contains(&"clean -purge".to_string()));
        assert_eq!(
            script.last().map(String::as_str),
            Some("write_json \"/tmp/netlist.json\"")
        );
    }

    #[test]
    fn displays_source_paths_relative_to_the_current_directory() {
        let current_dir = std::env::current_dir().expect("current directory");
        let source = current_dir.join("example/src/introduction.md");
        assert_eq!(
            PathBuf::from(display_source_path(&source)),
            PathBuf::from("example/src/introduction.md")
        );
    }

    #[test]
    fn supports_only_html_renderer() {
        let preprocessor = ClashPreprocessor;
        assert!(preprocessor.supports_renderer("html").unwrap());
        assert!(!preprocessor.supports_renderer("markdown").unwrap());
    }
}
