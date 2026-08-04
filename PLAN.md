# mdbook-clash plan

## Goal

Provide a Clash-focused mdBook preprocessor that keeps examples readable as
normal Haskell code blocks while making documentation executable:

- snippets with `>>>` examples are simulated at build time;
- snippets with `topEntity=...` are synthesized to Verilog at build time;
- snippets can be both simulated and synthesized from one block;
- failures point back to the chapter and code block that caused them.

## Implemented MVP

### User-facing behavior

1. Enable through `book.toml`:

   ```toml
   [preprocessor.clash]
   command = "mdbook-clash"
   clash-cmd = ["cabal", "run", "clash", "--"]
   ghc-cmd = ["cabal", "exec", "ghc", "--"]
   ```

2. Recognize fenced code blocks:

   ````md
   ```haskell,clash
   ...
   ```
   ````

   ````md
   ```haskell,clash topEntity=adder
   ...
   ```
   ````

3. Preserve the original code blocks in the rendered book.
4. Fail `mdbook build` if synthesis or simulation fails.

### Synthesis MVP

- `topEntity=...` implies synthesis.
- Wrap the snippet in a generated module.
- Generate the module name from book title, page stem, and block source line,
  e.g. `MyBook.Introduction.Line23`.
- Invoke:

  ```sh
  CLASH_CMD <generated-module> --verilog -outputdir <artifact-dir>/verilog
  ```

- Configurable:
  - `clash-cmd`, default `["clash"]`
  - `clash-args`, default `[]`
  - `work-dir`, default `mdbook-clash-work`
  - `keep-artifacts`, default `false`

### Simulation MVP

- `>>>` examples imply simulation.
- Generate a complete `Main` module around snippets.
- Put `Clash.Prelude` implicitly in scope.
- Support limited doctest syntax:

  ```haskell
  >>> expression
  expected-single-line-show-output
  ```

- Compile with:

  ```sh
  GHC_CMD <generated-main> -o <artifact-dir>/mdbook-clash-test
  ```

- Run the compiled executable.
- Configurable:
  - `ghc-cmd`, default `["ghc"]`
  - `ghc-args`, default `["-package", "clash-prelude"]`
  - `test-exe-args`, default `[]`

### Combined check MVP

- Support:

  ````md
  ```haskell,clash topEntity=increment
  increment :: Unsigned 8 -> Unsigned 8
  increment x = x + 1

  >>> increment 21
  22
  ```
  ````

- Run simulation for the doctest.
- Strip doctest prompt/expected-output lines from the synthesis module.
- Run synthesis because `topEntity=increment` is present.

### Caching MVP

- Cache successful checks by hashing:
  - chapter path;
  - block index and line;
  - code;
  - check phase;
  - block attributes;
  - Clash/GHC commands and args.
- Store a `success.json` marker in the artifact directory.
- Default `cache = true`.

### Diagnostics MVP

On failure, include:

- chapter path;
- code block index;
- generated file path;
- command;
- exit status;
- stdout and stderr.

### Repository contents

- Rust crate: `mdbook-clash`
- `shell.nix` with `cargo`, `rustc`, `rustfmt`, and `mdbook`
- example mdBook
- README with syntax and configuration

## Remaining production evolution

### Parser robustness

The simple scanner has been replaced with `pulldown-cmark` and offset-based
code-block extraction. Remaining work:

- richer quoted attribute parsing;
- exact column diagnostics;
- fixture tests for indented and nested Markdown edge cases.

### Rich block attributes

Support options per block:

```md
```haskell,clash topEntity=foo netlistsvg
```
```

Possible attributes:

- `topEntity=...`
- `hdl=verilog|vhdl|systemverilog`
- `timeout=...`
- `requires=...`
- `netlistsvg`
- `skip`
- `no-run`
- `keep-artifacts`
- `expect-fail`

### Snippet synthesis

The first version is implemented. Remaining work:

- validate `topEntity` syntax before generating files;
- support additional imports/extensions per block;
- optionally support `topEntityType=...` to force better type errors.

### Better doctests

Implement a real doctest parser:

- multiline expressions;
- multiline expected output;
- expected exceptions;
- wildcard matching;
- property-style checks;
- optional normalization for Clash values.

Long-term, consider generating a proper test module instead of emulating GHCi.

### Caching

The first success-marker cache is implemented. Remaining work:

- include plugin version in the hash;
- include relevant environment variables;
- write a richer manifest:

```json
{
  "hash": "...",
  "mode": "synth",
  "source": "src/tutorial.md",
  "block": 3,
  "status": "ok",
  "artifacts": ["verilog/..."]
}
```

### Yosys reports and netlistsvg

Implemented:

- Shell dependencies:
  - `yosys`
  - `netlistsvg`
- Config:
  - `yosys-cmd`, default `["yosys"]`
  - `netlistsvg-cmd`, default `["netlistsvg"]`
- Block attribute:
  - `netlistsvg`
  - `yosys="proc; opt; ..."` when combined with `netlistsvg`
- After Clash synthesis:
  - find generated Verilog files;
  - write a Yosys script that loads Verilog and selects `topEntity`;
  - insert `yosys="..."` commands when provided, otherwise use `proc; opt`;
  - always finish the netlist export with `clean -purge` and `write_json`;
  - reject `yosys="..."` without `netlistsvg`;
  - run `yosys -s <script>`;
  - run `netlistsvg <json> -o <svg>`;
  - inject the generated SVG into the documentation.

Remaining:

- Consider styling hooks for the generated netlist container.

### Parallelism

Run independent checks concurrently with a configured job count:

```toml
jobs = 4
```

Sensible production behavior:

- default to sequential for deterministic logs;
- allow parallelism in CI;
- preserve stable error ordering.

### Timeouts and sandboxing

Add per-block and global command timeouts. This is important because examples
can hang through non-terminating simulation or pathological synthesis.

Production documentation should clearly state that examples are trusted code.
This preprocessor is not a sandbox.

### CI integration

Provide a recommended CI job:

```sh
mdbook build docs
```

Add a `--check-only` mode later if there is demand, but mdBook preprocessors are
naturally tied to `mdbook build`.

### Artifact publishing

Potentially expose generated Verilog:

```md
{{#clash-artifact block="adder" file="topEntity.v"}}
```

This should be a separate feature from checking. The first production version
should avoid committing to artifact layout as public API.

### Version compatibility

Track compatibility with:

- mdBook preprocessor JSON format;
- Clash CLI flags;
- GHC versions used by supported Clash releases.

Pin Rust dependencies conservatively and keep the command invocation layer small.

## Non-goals

- Running untrusted code safely.
- Replacing Clash test suites.
- Inferring arbitrary synthesizable modules from arbitrary snippets without
  explicit user intent.
- Acting as a general Haskell doctest replacement.
