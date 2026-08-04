use mdbook_clash::ClashPreprocessor;
use mdbook_preprocessor::book::{Book, Chapter};
use mdbook_preprocessor::config::Config;
use mdbook_preprocessor::{Preprocessor, PreprocessorContext};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

fn make_context(root: &Path, clash_command: &Path, ghc_command: &Path) -> PreprocessorContext {
    let mut config = Config::default();
    config
        .set("book.title", "Test Book")
        .expect("set book title");
    config
        .set("preprocessor.clash.keep-artifacts", true)
        .expect("set keep-artifacts");
    config
        .set(
            "preprocessor.clash.clash-cmd",
            vec![clash_command.display().to_string()],
        )
        .expect("set clash-cmd");
    config
        .set(
            "preprocessor.clash.ghc-cmd",
            vec![ghc_command.display().to_string()],
        )
        .expect("set ghc-cmd");
    config
        .set("preprocessor.clash.ghc-args", Vec::<String>::new())
        .expect("set ghc-args");

    PreprocessorContext::new(root.to_path_buf(), config, "html".to_string())
}

fn make_context_with_yosys(
    root: &Path,
    clash_command: &Path,
    ghc_command: &Path,
    yosys_command: &Path,
) -> PreprocessorContext {
    let mut ctx = make_context(root, clash_command, ghc_command);
    ctx.config
        .set(
            "preprocessor.clash.yosys-cmd",
            vec![yosys_command.display().to_string()],
        )
        .expect("set yosys-cmd");
    ctx.config
        .set("preprocessor.clash.cache", false)
        .expect("disable cache");
    ctx
}

fn make_context_with_yosys_and_netlistsvg(
    root: &Path,
    clash_command: &Path,
    ghc_command: &Path,
    yosys_command: &Path,
    netlistsvg_command: &Path,
) -> PreprocessorContext {
    let mut ctx = make_context_with_yosys(root, clash_command, ghc_command, yosys_command);
    ctx.config
        .set(
            "preprocessor.clash.netlistsvg-cmd",
            vec![netlistsvg_command.display().to_string()],
        )
        .expect("set netlistsvg-cmd");
    ctx
}

fn make_context_with_cache(
    root: &Path,
    clash_command: &Path,
    ghc_command: &Path,
    cache: bool,
) -> PreprocessorContext {
    let mut ctx = make_context(root, clash_command, ghc_command);
    ctx.config
        .set("preprocessor.clash.cache", cache)
        .expect("set cache");
    ctx
}

fn make_book(content: &str) -> Book {
    let chapter = Chapter::new(
        "Integration",
        content.to_string(),
        PathBuf::from("src/integration.md"),
        Vec::new(),
    );
    Book::new_with_items(vec![chapter.into()])
}

fn write_executable(path: &Path, body: &str) {
    let shell = std::env::var_os("PATH")
        .and_then(|path| {
            std::env::split_paths(&path)
                .map(|directory| directory.join("sh"))
                .find(|candidate| candidate.is_file())
        })
        .expect("find sh on PATH");
    let body = body.replace("#!/usr/bin/env sh", &format!("#!{}", shell.display()));
    fs::write(path, body).expect("write fake executable");
    let mut permissions = fs::metadata(path)
        .expect("fake executable metadata")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).expect("chmod fake executable");
}

fn find_file(root: &Path, name: &str) -> PathBuf {
    for entry in fs::read_dir(root).expect("read directory") {
        let path = entry.expect("read directory entry").path();
        if path.is_dir() {
            let found = find_file(&path, name);
            if found.exists() {
                return found;
            }
        } else if path.file_name().and_then(|file| file.to_str()) == Some(name) {
            return path;
        }
    }
    PathBuf::new()
}

fn run_artifact_count(root: &Path) -> usize {
    fs::read_dir(root.join("mdbook-clash-work/runs"))
        .map(|entries| entries.count())
        .unwrap_or(0)
}

#[test]
fn supports_only_html_renderer() {
    let binary = env!("CARGO_BIN_EXE_mdbook-clash");
    assert!(Command::new(binary)
        .args(["supports", "html"])
        .status()
        .expect("run supports html")
        .success());
    assert!(!Command::new(binary)
        .args(["supports", "markdown"])
        .status()
        .expect("run supports markdown")
        .success());
}

#[test]
fn non_cached_runs_clean_up_without_deleting_existing_cache() {
    let temp = TempDir::new().expect("tempdir");
    let fake_clash = temp.path().join("fake-clash");
    let fake_ghc = temp.path().join("fake-ghc");
    let cache_marker = temp
        .path()
        .join("mdbook-clash-work/cache-v1/existing-cache");
    fs::create_dir_all(cache_marker.parent().expect("cache marker parent"))
        .expect("create existing cache directory");
    fs::write(&cache_marker, "keep").expect("write existing cache marker");
    write_executable(
        &fake_clash,
        r#"#!/usr/bin/env sh
set -eu
out=
while [ "$#" -gt 0 ]; do
  if [ "$1" = "-outputdir" ]; then
    shift
    out="$1"
  fi
  shift || true
done
mkdir -p "$out"
printf 'module example(); endmodule\n' > "$out/example.v"
printf '{"top_component":{"name":"example"}}\n' > "$out/clash-manifest.json"
"#,
    );
    write_executable(&fake_ghc, "#!/usr/bin/env sh\nexit 0\n");

    let mut ctx = make_context_with_cache(temp.path(), &fake_clash, &fake_ghc, false);
    ctx.config
        .set("preprocessor.clash.keep-artifacts", false)
        .expect("disable artifact retention");
    let book = || make_book("```haskell,clash topEntity=example\nexample = id\n```");

    ClashPreprocessor.run(&ctx, book()).expect("successful run");
    assert!(cache_marker.exists());
    assert_eq!(run_artifact_count(temp.path()), 0);

    write_executable(&fake_clash, "#!/usr/bin/env sh\nexit 17\n");
    ClashPreprocessor
        .run(&ctx, book())
        .expect_err("failing run");
    assert!(cache_marker.exists());
    assert_eq!(run_artifact_count(temp.path()), 0);
}

#[test]
fn synth_blocks_invoke_configured_clash_command() {
    let temp = TempDir::new().expect("tempdir");
    let calls = temp.path().join("calls.txt");
    let fake_clash = temp.path().join("fake-clash");
    let fake_ghc = temp.path().join("fake-ghc");
    write_executable(
        &fake_clash,
        &format!(
            r#"#!/usr/bin/env sh
set -eu
printf '%s\n' "$@" > '{}'
while [ "$#" -gt 0 ]; do
  if [ "$1" = "-outputdir" ]; then
    shift
    mkdir -p "$1"
    printf 'module adder(); endmodule\n' > "$1/adder.v"
    printf '{{"top_component":{{"name":"adder"}}}}\n' > "$1/clash-manifest.json"
  fi
  shift || true
done
"#,
            calls.display()
        ),
    );
    write_executable(&fake_ghc, "#!/usr/bin/env sh\nexit 0\n");

    let ctx = make_context(temp.path(), &fake_clash, &fake_ghc);
    let book = make_book(
        r#"
```haskell,clash topEntity=adder
adder :: Unsigned 8 -> Unsigned 8 -> Unsigned 8
adder a b = a + b
```
"#,
    );

    ClashPreprocessor
        .run(&ctx, book)
        .expect("preprocessor succeeds");

    let call = fs::read_to_string(calls).expect("read fake clash call");
    assert!(call.contains("TestBook/Integration/Line2.hs"), "{call}");
    assert!(call.contains("--verilog"), "{call}");
    assert!(call.contains("-outputdir"), "{call}");
    assert!(call.contains("verilog"), "{call}");
}

#[test]
fn test_blocks_invoke_runner_with_implicit_clash_prelude_wrapper() {
    let temp = TempDir::new().expect("tempdir");
    let calls = temp.path().join("calls.txt");
    let copied_main = temp.path().join("generated-main.hs");
    let fake_clash = temp.path().join("fake-clash");
    let fake_ghc = temp.path().join("fake-ghc");
    write_executable(&fake_clash, "#!/usr/bin/env sh\nexit 0\n");
    write_executable(
        &fake_ghc,
        &format!(
            r#"#!/usr/bin/env sh
set -eu
printf '%s\n' "$@" > '{}'
src=
out=
while [ "$#" -gt 0 ]; do
  case "$1" in
    -o)
      shift
      out="$1"
      ;;
    *.hs)
      src="$1"
      ;;
  esac
  shift || true
done
cp "$src" '{}'
printf '#!/usr/bin/env sh\nexit 0\n' > "$out"
chmod +x "$out"
"#,
            calls.display(),
            copied_main.display()
        ),
    );

    let ctx = make_context(temp.path(), &fake_clash, &fake_ghc);
    let book = make_book(
        r#"
```haskell,clash
double :: Unsigned 8 -> Unsigned 8
double x = x + x

>>> double 10
20
```
"#,
    );

    ClashPreprocessor
        .run(&ctx, book)
        .expect("preprocessor succeeds");

    let call = fs::read_to_string(calls).expect("read fake runner call");
    assert!(call.contains("Main.hs"), "{call}");
    assert!(call.contains("-o"), "{call}");

    let generated = fs::read_to_string(copied_main).expect("read generated Main.hs");
    assert!(generated.contains("import Clash.Prelude"), "{generated}");
    assert!(
        generated.contains("__mdbookClashAssertEqual \"doctest 1\" (20) (double 10)"),
        "{generated}"
    );
    assert!(!generated.contains("double 21"), "{generated}");
}

#[test]
fn command_failures_report_actionable_diagnostics() {
    let temp = TempDir::new().expect("tempdir");
    let calls = temp.path().join("clash-calls.txt");
    let fake_clash = temp.path().join("fake-failing-clash");
    let fake_ghc = temp.path().join("fake-ghc");
    write_executable(
        &fake_clash,
        &format!(
            r#"#!/usr/bin/env sh
set -eu
printf 'call\n' >> '{}'
echo "fake stdout"
echo "fake stderr" >&2
exit 17
"#,
            calls.display()
        ),
    );
    write_executable(&fake_ghc, "#!/usr/bin/env sh\nexit 0\n");

    let ctx = make_context(temp.path(), &fake_clash, &fake_ghc);
    let markdown = r#"
```haskell,clash topEntity=broken
broken :: Unsigned 8 -> Unsigned 8
broken = id
```
"#;

    let err = ClashPreprocessor
        .run(&ctx, make_book(markdown))
        .expect_err("preprocessor should fail");
    let err = err.to_string();

    assert!(err.contains("synthesis failed"), "{err}");
    assert!(err.contains("src/integration.md"), "{err}");
    assert!(err.contains("block: 1"), "{err}");
    assert!(err.contains("generated:"), "{err}");
    assert!(err.contains("fake stdout"), "{err}");
    assert!(err.contains("fake stderr"), "{err}");

    ClashPreprocessor
        .run(&ctx, make_book(markdown))
        .expect_err("failed synthesis must not be cached");
    assert_eq!(
        fs::read_to_string(calls)
            .expect("read Clash calls")
            .lines()
            .count(),
        2
    );
}

#[test]
fn simulation_failures_are_not_cached() {
    let temp = TempDir::new().expect("tempdir");
    let calls = temp.path().join("test-calls.txt");
    let fake_clash = temp.path().join("fake-clash");
    let fake_ghc = temp.path().join("fake-ghc");
    write_executable(&fake_clash, "#!/usr/bin/env sh\nexit 0\n");
    write_executable(
        &fake_ghc,
        &format!(
            r#"#!/usr/bin/env sh
set -eu
printf 'call\n' >> '{}'
out=
while [ "$#" -gt 0 ]; do
  if [ "$1" = "-o" ]; then
    shift
    out="$1"
  fi
  shift || true
done
printf '#!/usr/bin/env sh\necho simulation-failed >&2\nexit 17\n' > "$out"
chmod +x "$out"
"#,
            calls.display()
        ),
    );

    let ctx = make_context(temp.path(), &fake_clash, &fake_ghc);
    let book = || make_book("```haskell,clash\n>>> id 1\n1\n```");

    for _ in 0..2 {
        let err = ClashPreprocessor
            .run(&ctx, book())
            .expect_err("simulation should fail")
            .to_string();
        assert!(err.contains("simulation failed"), "{err}");
        assert!(err.contains("simulation-failed"), "{err}");
    }
    assert_eq!(
        fs::read_to_string(calls)
            .expect("read test calls")
            .lines()
            .count(),
        2
    );
}

#[test]
fn clash_blocks_with_top_entity_are_wrapped_and_synthesized() {
    let temp = TempDir::new().expect("tempdir");
    let calls = temp.path().join("calls.txt");
    let copied_source = temp.path().join("generated-source.hs");
    let fake_clash = temp.path().join("fake-clash");
    let fake_ghc = temp.path().join("fake-ghc");
    write_executable(
        &fake_clash,
        &format!(
            r#"#!/usr/bin/env sh
set -eu
printf '%s\n' "$@" > '{}'
src=
out=
while [ "$#" -gt 0 ]; do
  case "$1" in
    *.hs)
      src="$1"
      ;;
    -outputdir)
      shift
      out="$1"
      ;;
  esac
  shift || true
done
cp "$src" '{}'
mkdir -p "$out"
printf 'module adder(); endmodule\n' > "$out/adder.v"
printf '{{"top_component":{{"name":"adder"}}}}\n' > "$out/clash-manifest.json"
"#,
            calls.display(),
            copied_source.display()
        ),
    );
    write_executable(&fake_ghc, "#!/usr/bin/env sh\nexit 0\n");

    let ctx = make_context(temp.path(), &fake_clash, &fake_ghc);
    let book = make_book(
        r#"
~~~haskell,clash topEntity=adder
adder :: Unsigned 8 -> Unsigned 8 -> Unsigned 8
adder a b = a + b
~~~
"#,
    );

    ClashPreprocessor
        .run(&ctx, book)
        .expect("preprocessor succeeds");

    let call = fs::read_to_string(calls).expect("read fake clash call");
    assert!(call.contains("TestBook/Integration/Line2.hs"), "{call}");

    let generated = fs::read_to_string(copied_source).expect("read generated snippet source");
    assert!(
        generated.contains("module TestBook.Integration.Line2 where"),
        "{generated}"
    );
    assert!(generated.contains("import Clash.Prelude"), "{generated}");
    assert!(generated.contains("adder a b = a + b"), "{generated}");
    assert!(!generated.contains("topEntity = adder"), "{generated}");
    assert!(call.contains("-main-is\nadder"), "{call}");
}

#[test]
fn cache_keys_are_phase_specific_and_cached_outputs_are_verified() {
    let temp = TempDir::new().expect("tempdir");
    let calls = temp.path().join("calls.txt");
    let fake_clash = temp.path().join("fake-clash");
    let fake_ghc = temp.path().join("fake-ghc");
    write_executable(
        &fake_clash,
        &format!(
            r#"#!/usr/bin/env sh
set -eu
printf 'call\n' >> '{}'
out=
while [ "$#" -gt 0 ]; do
  if [ "$1" = "-outputdir" ]; then
    shift
    out="$1"
  fi
  shift || true
done
mkdir -p "$out"
printf 'module cached(); endmodule\n' > "$out/cached.v"
printf '{{"top_component":{{"name":"cached"}}}}\n' > "$out/clash-manifest.json"
"#,
            calls.display()
        ),
    );
    write_executable(&fake_ghc, "#!/usr/bin/env sh\nexit 0\n");

    let mut ctx = make_context_with_cache(temp.path(), &fake_clash, &fake_ghc, true);
    let content = r#"
```haskell,clash topEntity=cached
cached :: Unsigned 8 -> Unsigned 8
cached = id
```
"#;

    ClashPreprocessor
        .run(&ctx, make_book(content))
        .expect("first run succeeds");
    ClashPreprocessor
        .run(&ctx, make_book(content))
        .expect("second run succeeds");

    ctx.config
        .set("preprocessor.clash.yosys-cmd", vec!["different-yosys"])
        .expect("change unrelated command");
    ClashPreprocessor
        .run(&ctx, make_book(content))
        .expect("unrelated config change keeps synthesis cache valid");

    let call_count = || {
        fs::read_to_string(&calls)
            .expect("read call log")
            .lines()
            .count()
    };
    assert_eq!(call_count(), 1);

    fs::write(
        find_file(&temp.path().join("mdbook-clash-work"), "cached.v"),
        "corrupt",
    )
    .expect("corrupt cached output");
    ClashPreprocessor
        .run(&ctx, make_book(content))
        .expect("corrupt cache entry is rebuilt");
    assert_eq!(call_count(), 2);

    ctx.config
        .set("preprocessor.clash.cache-key", "new-toolchain")
        .expect("change cache key");
    ClashPreprocessor
        .run(&ctx, make_book(content))
        .expect("new user cache key rebuilds synthesis");
    assert_eq!(call_count(), 3);
}

#[test]
fn invalid_configuration_is_an_error() {
    let temp = TempDir::new().expect("tempdir");
    let fake_clash = temp.path().join("fake-clash");
    let fake_ghc = temp.path().join("fake-ghc");
    write_executable(&fake_clash, "#!/usr/bin/env sh\nexit 0\n");
    write_executable(&fake_ghc, "#!/usr/bin/env sh\nexit 0\n");

    let mut ctx = make_context(temp.path(), &fake_clash, &fake_ghc);
    ctx.config
        .set("preprocessor.clash.clash-cmd", "clash --verilog")
        .expect("set invalid command type");
    let err = ClashPreprocessor
        .run(
            &ctx,
            make_book("```haskell,clash topEntity=example\nexample = id\n```"),
        )
        .expect_err("invalid config should fail")
        .to_string();

    assert!(
        err.contains("invalid `preprocessor.clash.clash-cmd`"),
        "{err}"
    );

    let mut ctx = make_context(temp.path(), &fake_clash, &fake_ghc);
    ctx.config
        .set("preprocessor.clash.work-dir", "../outside")
        .expect("set unsafe work directory");
    let err = ClashPreprocessor
        .run(&ctx, make_book("plain text"))
        .expect_err("unsafe work directory should fail")
        .to_string();
    assert!(err.contains("must be a non-empty relative path"), "{err}");
}

#[test]
fn invalid_attributes_are_rejected_before_doctests_run() {
    let temp = TempDir::new().expect("tempdir");
    let calls = temp.path().join("ghc-calls.txt");
    let fake_clash = temp.path().join("fake-clash");
    let fake_ghc = temp.path().join("fake-ghc");
    write_executable(&fake_clash, "#!/usr/bin/env sh\nexit 0\n");
    write_executable(
        &fake_ghc,
        &format!("#!/usr/bin/env sh\nprintf called > '{}'\n", calls.display()),
    );
    let ctx = make_context(temp.path(), &fake_clash, &fake_ghc);

    let err = ClashPreprocessor
        .run(
            &ctx,
            make_book("```haskell,clash netlistsvg\n>>> id 1\n1\n```"),
        )
        .expect_err("netlistsvg without a top entity should fail")
        .to_string();
    assert!(err.contains("netlistsvg requires topEntity"), "{err}");
    assert!(!calls.exists(), "validation must happen before execution");

    let err = ClashPreprocessor
        .run(
            &ctx,
            make_book("```haskell,clash topEntity=example unknown\nexample = id\n```"),
        )
        .expect_err("unknown attributes should fail")
        .to_string();
    assert!(err.contains("unknown attribute `unknown`"), "{err}");

    let err = ClashPreprocessor
        .run(
            &ctx,
            make_book("```haskell,clash topEntity=example yosys=\"proc\nexample = id\n```"),
        )
        .expect_err("malformed quoting should fail")
        .to_string();
    assert!(err.contains("invalid fenced block attributes"), "{err}");
}

#[test]
fn yosys_script_attribute_requires_netlistsvg() {
    let temp = TempDir::new().expect("tempdir");
    let fake_clash = temp.path().join("fake-clash");
    let fake_ghc = temp.path().join("fake-ghc");

    write_executable(
        &fake_clash,
        r#"#!/usr/bin/env sh
set -eu
while [ "$#" -gt 0 ]; do
  if [ "$1" = "-outputdir" ]; then
    shift
    mkdir -p "$1"
    printf 'module adder(input [7:0] a, output [7:0] b); assign b = a; endmodule\n' > "$1/adder.v"
    printf '{"top_component":{"name":"adder"}}\n' > "$1/clash-manifest.json"
  fi
  shift || true
done
"#,
    );
    write_executable(&fake_ghc, "#!/usr/bin/env sh\nexit 0\n");

    let ctx = make_context(temp.path(), &fake_clash, &fake_ghc);
    let book = make_book(
        r#"
```haskell,clash topEntity=adder yosys="proc; opt" netlistsvg
adder :: Unsigned 8 -> Unsigned 8 -> Unsigned 8
adder a b = a + b
```
"#,
    );
    let ok_ctx = make_context_with_yosys_and_netlistsvg(
        temp.path(),
        &fake_clash,
        &fake_ghc,
        &temp.path().join("unused-yosys"),
        &temp.path().join("unused-netlistsvg"),
    );
    let standalone_book = make_book(
        r#"
```haskell,clash topEntity=adder yosys="proc; opt"
adder :: Unsigned 8 -> Unsigned 8 -> Unsigned 8
adder a b = a + b
```
"#,
    );

    let err = ClashPreprocessor
        .run(&ctx, standalone_book)
        .expect_err("preprocessor should reject standalone yosys attribute");
    let err = err.to_string();

    assert!(
        err.contains("yosys=<commands> requires netlistsvg"),
        "{err}"
    );
    assert!(
        err.contains("Yosys commands are only used to generate netlist diagrams"),
        "{err}"
    );

    let err = ClashPreprocessor
        .run(&ok_ctx, book)
        .expect_err("fake commands are missing, but yosys attribute itself is accepted")
        .to_string();
    assert!(
        !err.contains("yosys=<commands> requires netlistsvg"),
        "{err}"
    );
}

#[test]
fn netlistsvg_runs_yosys_json_export_and_injects_svg() {
    let temp = TempDir::new().expect("tempdir");
    let yosys_script_copy = temp.path().join("netlist.ys");
    let netlistsvg_calls = temp.path().join("netlistsvg-calls.txt");
    let clash_verilog = temp.path().join("clash-output.v");
    let fake_clash = temp.path().join("fake-clash");
    let fake_ghc = temp.path().join("fake-ghc");
    let fake_yosys = temp.path().join("fake-yosys");
    let fake_netlistsvg = temp.path().join("fake-netlistsvg");

    fs::write(
        &clash_verilog,
        "module custom_adder(input a, output b); assign b = a; endmodule\n",
    )
    .expect("write fake Clash output");
    write_executable(
        &fake_clash,
        &format!(
            r#"#!/usr/bin/env sh
set -eu
while [ "$#" -gt 0 ]; do
  if [ "$1" = "-outputdir" ]; then
    shift
    mkdir -p "$1"
    cp '{}' "$1/custom_adder.v"
    printf '{{"top_component":{{"name":"custom_adder"}}}}\n' > "$1/clash-manifest.json"
  fi
  shift || true
done
"#,
            clash_verilog.display()
        ),
    );
    write_executable(&fake_ghc, "#!/usr/bin/env sh\nexit 0\n");
    write_executable(
        &fake_yosys,
        &format!(
            r#"#!/usr/bin/env sh
set -eu
script=
while [ "$#" -gt 0 ]; do
  if [ "$1" = "-s" ]; then
    shift
    script="$1"
  fi
  shift || true
done
cp "$script" '{}'
json=$(sed -n 's/^write_json "\(.*\)"$/\1/p' "$script")
printf '{{"modules":{{"topEntity":{{}}}}}}\n' > "$json"
"#,
            yosys_script_copy.display()
        ),
    );
    write_executable(
        &fake_netlistsvg,
        &format!(
            r#"#!/usr/bin/env sh
set -eu
printf 'call\n' >> '{}'
out=
while [ "$#" -gt 0 ]; do
  if [ "$1" = "-o" ]; then
    shift
    out="$1"
  fi
  shift || true
done
printf '<svg xmlns="http://www.w3.org/2000/svg"><text>adder netlist</text></svg>\n' > "$out"
"#,
            netlistsvg_calls.display()
        ),
    );

    let mut ctx = make_context_with_yosys_and_netlistsvg(
        temp.path(),
        &fake_clash,
        &fake_ghc,
        &fake_yosys,
        &fake_netlistsvg,
    );
    ctx.config
        .set("preprocessor.clash.cache", true)
        .expect("enable cache");
    let markdown = r#"
```haskell,clash topEntity=adder yosys="proc; opt; techmap" netlistsvg
adder :: Bit -> Bit
adder x = x
```
"#;

    let out = ClashPreprocessor
        .run(&ctx, make_book(markdown))
        .expect("preprocessor succeeds");
    let rendered = &out.chapters().next().expect("chapter").content;
    let script = fs::read_to_string(yosys_script_copy).expect("read copied Yosys script");

    assert!(script.contains("read_verilog"), "{script}");
    assert!(script.contains("hierarchy -top custom_adder"), "{script}");
    assert!(script.contains("proc"), "{script}");
    assert!(script.contains("opt"), "{script}");
    assert!(script.contains("techmap"), "{script}");
    assert!(script.contains("clean -purge"), "{script}");
    assert!(script.contains("write_json"), "{script}");
    assert!(rendered.contains("#### Netlist"), "{rendered}");
    assert!(
        rendered.contains(r#"style="background: white;"#),
        "{rendered}"
    );
    assert!(rendered.contains("<svg"), "{rendered}");
    assert!(rendered.contains("adder netlist"), "{rendered}");
    assert!(!rendered.contains("write_json"), "{rendered}");
    assert!(
        !rendered.contains(&temp.path().display().to_string()),
        "{rendered}"
    );

    fs::write(
        &clash_verilog,
        "module custom_adder(input a, output b); assign b = ~a; endmodule\n",
    )
    .expect("change fake Clash output");
    fs::write(
        find_file(&temp.path().join("mdbook-clash-work"), "custom_adder.v"),
        "corrupt",
    )
    .expect("invalidate synthesis cache");
    ClashPreprocessor
        .run(&ctx, make_book(markdown))
        .expect("changed synthesis output rebuilds netlist");
    assert_eq!(
        fs::read_to_string(netlistsvg_calls)
            .expect("read netlistsvg calls")
            .lines()
            .count(),
        2
    );
}

#[test]
fn missing_netlistsvg_reports_setup_hint() {
    let temp = TempDir::new().expect("tempdir");
    let yosys_calls = temp.path().join("yosys-calls.txt");
    let fake_clash = temp.path().join("fake-clash");
    let fake_ghc = temp.path().join("fake-ghc");
    let fake_yosys = temp.path().join("fake-yosys");
    let missing_netlistsvg = temp.path().join("missing-netlistsvg");

    write_executable(
        &fake_clash,
        r#"#!/usr/bin/env sh
set -eu
while [ "$#" -gt 0 ]; do
  if [ "$1" = "-outputdir" ]; then
    shift
    mkdir -p "$1"
    printf 'module wire(input a, output b); assign b = a; endmodule\n' > "$1/wire.v"
    printf '{"top_component":{"name":"wire"}}\n' > "$1/clash-manifest.json"
  fi
  shift || true
done
"#,
    );
    write_executable(&fake_ghc, "#!/usr/bin/env sh\nexit 0\n");
    write_executable(
        &fake_yosys,
        &format!(
            r#"#!/usr/bin/env sh
set -eu
printf 'call\n' >> '{}'
script=
while [ "$#" -gt 0 ]; do
  if [ "$1" = "-s" ]; then
    shift
    script="$1"
  fi
  shift || true
done
json=$(sed -n 's/^write_json "\(.*\)"$/\1/p' "$script")
printf '{{"modules":{{"topEntity":{{}}}}}}\n' > "$json"
"#,
            yosys_calls.display()
        ),
    );

    let mut ctx = make_context_with_yosys_and_netlistsvg(
        temp.path(),
        &fake_clash,
        &fake_ghc,
        &fake_yosys,
        &missing_netlistsvg,
    );
    ctx.config
        .set("preprocessor.clash.cache", true)
        .expect("enable cache");
    let markdown = r#"
```haskell,clash topEntity=wire netlistsvg
wire :: Bit -> Bit
wire x = x
```
"#;

    for _ in 0..2 {
        let err = ClashPreprocessor
            .run(&ctx, make_book(markdown))
            .expect_err("preprocessor should fail")
            .to_string();
        assert!(err.contains("failed to start netlistsvg command"), "{err}");
        assert!(err.contains("netlistsvg` was requested"), "{err}");
        assert!(err.contains("netlistsvg-cmd"), "{err}");
        assert!(
            err.contains(&missing_netlistsvg.display().to_string()),
            "{err}"
        );
    }
    assert_eq!(
        fs::read_to_string(yosys_calls)
            .expect("read Yosys calls")
            .lines()
            .count(),
        2,
        "failed netlist generation must not be cached",
    );
}

#[test]
fn clash_blocks_can_be_simulated_and_synthesized() {
    let temp = TempDir::new().expect("tempdir");
    let clash_calls = temp.path().join("clash-calls.txt");
    let ghc_calls = temp.path().join("ghc-calls.txt");
    let copied_synth_source = temp.path().join("synth-source.hs");
    let copied_main = temp.path().join("main-source.hs");
    let fake_clash = temp.path().join("fake-clash");
    let fake_ghc = temp.path().join("fake-ghc");

    write_executable(
        &fake_clash,
        &format!(
            r#"#!/usr/bin/env sh
set -eu
printf 'clash\n' >> '{}'
src=
out=
while [ "$#" -gt 0 ]; do
  case "$1" in
    *.hs)
      src="$1"
      ;;
    -outputdir)
      shift
      out="$1"
      ;;
  esac
  shift || true
done
cp "$src" '{}'
mkdir -p "$out"
printf 'module increment(); endmodule\n' > "$out/increment.v"
printf '{{"top_component":{{"name":"increment"}}}}\n' > "$out/clash-manifest.json"
"#,
            clash_calls.display(),
            copied_synth_source.display()
        ),
    );
    write_executable(
        &fake_ghc,
        &format!(
            r#"#!/usr/bin/env sh
set -eu
printf 'ghc\n' >> '{}'
src=
out=
while [ "$#" -gt 0 ]; do
  case "$1" in
    -o)
      shift
      out="$1"
      ;;
    *.hs)
      src="$1"
      ;;
  esac
  shift || true
done
cp "$src" '{}'
printf '#!/usr/bin/env sh\nexit 0\n' > "$out"
chmod +x "$out"
"#,
            ghc_calls.display(),
            copied_main.display()
        ),
    );

    let ctx = make_context(temp.path(), &fake_clash, &fake_ghc);
    let book = make_book(
        r#"
```haskell,clash topEntity=increment
increment :: Unsigned 8 -> Unsigned 8
increment x = x + 1

>>> increment 21
22
```
"#,
    );

    ClashPreprocessor
        .run(&ctx, book)
        .expect("preprocessor succeeds");

    assert_eq!(
        fs::read_to_string(clash_calls)
            .expect("read clash calls")
            .lines()
            .count(),
        1
    );
    assert_eq!(
        fs::read_to_string(ghc_calls)
            .expect("read ghc calls")
            .lines()
            .count(),
        1
    );

    let synth_source = fs::read_to_string(copied_synth_source).expect("read synth source");
    assert!(
        !synth_source.contains("topEntity = increment"),
        "{synth_source}"
    );
    assert!(!synth_source.contains(">>>"), "{synth_source}");
    assert!(!synth_source.contains("22"), "{synth_source}");

    let main_source = fs::read_to_string(copied_main).expect("read main source");
    assert!(
        main_source.contains("__mdbookClashAssertEqual \"doctest 1\" (22) (increment 21)"),
        "{main_source}"
    );
}

#[test]
fn grouped_blocks_compile_once_run_per_doctest_block_and_synthesize_independently() {
    let temp = TempDir::new().expect("tempdir");
    let ghc_calls = temp.path().join("ghc-calls.txt");
    let simulation_calls = temp.path().join("simulation-calls.txt");
    let generated_main = temp.path().join("group-main.hs");
    let clash_calls = temp.path().join("clash-calls.txt");
    let synth_prefix = temp.path().join("synth-source");
    let fake_clash = temp.path().join("fake-clash");
    let fake_ghc = temp.path().join("fake-ghc");

    write_executable(
        &fake_ghc,
        &format!(
            r#"#!/usr/bin/env sh
set -eu
printf 'ghc\n' >> '{}'
src=
out=
while [ "$#" -gt 0 ]; do
  case "$1" in
    -o)
      shift
      out="$1"
      ;;
    *.hs)
      src="$1"
      ;;
  esac
  shift || true
done
cp "$src" '{}'
printf '#!/usr/bin/env sh\nprintf "%%s\\n" "$MDBOOK_CLASH_BLOCK" >> "{}"\n' > "$out"
chmod +x "$out"
"#,
            ghc_calls.display(),
            generated_main.display(),
            simulation_calls.display(),
        ),
    );
    write_executable(
        &fake_clash,
        &format!(
            r#"#!/usr/bin/env sh
set -eu
printf 'clash\n' >> '{}'
src=
out=
top=
while [ "$#" -gt 0 ]; do
  case "$1" in
    *.hs)
      src="$1"
      ;;
    -main-is)
      shift
      top="$1"
      ;;
    -outputdir)
      shift
      out="$1"
      ;;
  esac
  shift || true
done
cp "$src" '{}-'"$top"'.hs'
mkdir -p "$out"
printf 'module %s(); endmodule\n' "$top" > "$out/$top.v"
printf '{{"top_component":{{"name":"%s"}}}}\n' "$top" > "$out/clash-manifest.json"
"#,
            clash_calls.display(),
            synth_prefix.display(),
        ),
    );

    let ctx = make_context(temp.path(), &fake_clash, &fake_ghc);
    let book = make_book(
        r#"
```haskell,clash group=counter
offset :: Unsigned 8
offset = 1
```

```haskell,clash id=counter topEntity=increment
increment x = x + offset

>>> increment 4
5
```

```haskell,clash group=counter topEntity=decrement
decrement x = x - offset

>>> decrement 4
3
```
"#,
    );

    ClashPreprocessor
        .run(&ctx, book)
        .expect("grouped blocks should succeed");

    assert_eq!(
        fs::read_to_string(&ghc_calls)
            .expect("read GHC calls")
            .lines()
            .count(),
        1,
        "a group should be compiled once"
    );
    assert_eq!(
        fs::read_to_string(&simulation_calls)
            .expect("read simulation calls")
            .lines()
            .collect::<Vec<_>>(),
        ["2", "3"],
        "each doctest block should be run separately"
    );
    assert_eq!(
        fs::read_to_string(&clash_calls)
            .expect("read Clash calls")
            .lines()
            .count(),
        2,
        "each top entity should be synthesized separately"
    );

    let main = fs::read_to_string(generated_main).expect("read grouped Main module");
    assert!(main.contains("offset = 1"), "{main}");
    assert!(main.contains("increment x = x + offset"), "{main}");
    assert!(main.contains("decrement x = x - offset"), "{main}");
    assert!(main.contains("P.Just \"2\" -> do"), "{main}");
    assert!(main.contains("P.Just \"3\" -> do"), "{main}");

    for top in ["increment", "decrement"] {
        let source = fs::read_to_string(temp.path().join(format!("synth-source-{top}.hs")))
            .expect("read grouped synthesis module");
        assert!(source.contains("offset = 1"), "{source}");
        assert!(source.contains("increment x = x + offset"), "{source}");
        assert!(source.contains("decrement x = x - offset"), "{source}");
        assert!(!source.contains(">>>"), "{source}");
    }
}
