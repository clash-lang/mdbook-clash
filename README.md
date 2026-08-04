# mdbook-clash

`mdbook-clash` is an mdBook preprocessor for documentation examples written in
Clash/Haskell. It checks selected fenced code blocks during `mdbook build`.
It currently supports mdBook's HTML renderer.

The current implementation uses one fenced-code attribute:

- `clash`: check a Clash/Haskell snippet.

Within a `clash` block:

- `>>>` doctest examples trigger simulation.
- `topEntity=...` triggers synthesis to Verilog.
- `group=...` combines definitions from multiple blocks.
- `hidden` includes a grouped block in checks but removes it from the book.
- If both are present, both checks run.

## Installation

From this directory:

```sh
nix develop
cargo build --release
```

Then put `target/release/mdbook-clash` on `PATH`, or reference it directly from
`book.toml`. Simulation also requires `mdbook-clash-doctest` from the selected
Clash/GHC package set. The development shell and wrapped flake package provide
both executables together, which is the recommended installation method.

The flake also exposes a wrapped package containing all runtime dependencies:

```sh
nix run . -- supports html
nix build .
```

The development shell contains Rust, mdBook, Clash, doctest, Yosys, and
netlistsvg.

### Selecting a Clash release

The `clash-compiler` input is intentionally independent. To use another Clash
tag without editing this flake, update that input in the lock file:

```sh
nix flake lock \
  --override-input clash-compiler github:clash-lang/clash-compiler/v1.8.5
```

The `nixpkgs` input follows `clash-compiler/nixpkgs`, keeping the development
shell aligned with the selected Clash release. Downstream flakes can make the
same choice declaratively and have this flake follow it:

```nix
inputs.clash-compiler.url = "github:clash-lang/clash-compiler/v1.8.5";
inputs.mdbook-clash.url = "path:../mdbook-clash";
inputs.mdbook-clash.inputs.clash-compiler.follows = "clash-compiler";
```

For further composition, `overlays.default` and
`packages.<system>.mdbook-clash` are exported.

## mdBook configuration

With the flake development shell or wrapped package, use:

```toml
[preprocessor.clash]
command = "mdbook-clash"
clash-cmd = ["clash"]
```

If `clash-cmd` is omitted, it defaults to:

```toml
clash-cmd = ["clash"]
```

Other options:

```toml
[preprocessor.clash]
work-dir = "mdbook-clash-work"
keep-artifacts = false
cache = true
cache-key = ""
clash-args = ["-fclash-clear"]
yosys-cmd = ["yosys"]
netlistsvg-cmd = ["netlistsvg"]
```

`work-dir` is relative to the book root. Configuration values are type-checked;
commands must be non-empty TOML arrays.

Synthesis invokes `clash-cmd` directly. Simulation asks
`mdbook-clash-doctest` to use `clash-cmd` as its interactive REPL. Both phases
receive `clash-args`, so Clash's normal default language extensions apply to
the user module in both phases. `mdbook-clash-doctest` is a fixed companion
executable shipped with `mdbook-clash`; it is not separately configurable.

Caching uses independent keys for simulation, synthesis, and netlist rendering.
Each key covers the generated input, relevant command and arguments, the command
executable, the running `mdbook-clash` binary, and the cache format version.
Cached output files are content-hashed and verified before reuse. Incomplete,
modified, or malformed entries are discarded and rebuilt. Per-entry locks make
a shared work directory safe for concurrent builds.

Set `cache-key` when behavior can change without changing the configured
executable, for example when `clash-cmd` invokes `cabal run` and the Cabal
project changes. CI can set it to its toolchain or lock-file revision. Set
`cache = false` to disable reuse entirely.

## Simulation examples

The checked source owns its module declaration, language pragmas, and imports.
A hidden grouped block is a convenient place for boilerplate that should not be
shown in the rendered book:

````md
```haskell,clash group=double hidden
module DoubleExample where

import Clash.Prelude
```

```haskell,clash group=double
double :: Unsigned 8 -> Unsigned 8
double x = x + x

>>> double 21
42
```
````

mdbook-clash does not add a module declaration, pragmas, or imports to this
source. It uses upstream Haskell doctest for parsing, output matching, and
execution. Supported syntax includes multiline input and output,
`<BLANKLINE>`, `...` wildcards, sequential interactions, and `prop>`
properties.

The first `>>>` or `prop>` starts the doctest document for a fenced block. The
document continues to the end of that fence. Put any following definitions in
a later block in the same group. This gives doctest an unambiguous document
boundary without reimplementing its transcript parser in Rust.

## Grouped code blocks

Give several blocks the same `group` identifier when an example is easier to
explain in stages but should be checked as one program. Groups are local to a
chapter.

````md
```haskell,clash group=counter hidden
module CounterExample where

import Clash.Prelude
```

```haskell,clash group=counter
offset :: Unsigned 8
offset = 1
```

```haskell,clash group=counter topEntity=increment
increment x = x + offset

>>> increment 4
5
```

```haskell,clash group=counter topEntity=decrement
decrement x = x - offset

>>> decrement 4
3
```
````

The blocks are concatenated in chapter order. Simulation loads the complete
group into one Clash interpreter. Each fenced block containing a doctest is a
separate doctest example group, so interpreter state is reset between blocks
and retained between interactions within one block. Diagnostics retain the
Markdown source path and line. Each block with `topEntity=...` is synthesized
separately from the complete group, and its Yosys/netlistsvg options apply only
to that block. A `hidden` block is removed from the rendered Markdown but stays
in the concatenated source. Using `hidden` without `group=...` is an error.

Every visible grouped block links to a complete, copyable Haskell listing at the
end of the chapter. The listing concatenates the group in chapter order,
includes hidden source, and removes doctest prompts and expected output.

Grouped blocks are concatenated without rewriting their Haskell. Put module
headers, pragmas, and imports in the first block (usually a hidden setup block)
so the combined source remains a valid module.

## Synthesis examples

Add `topEntity=...` to synthesize a binding. The module and imports still come
from the checked source:

````md
```haskell,clash group=adder hidden
module AdderExample where

import Clash.Prelude
```

```haskell,clash group=adder topEntity=adder
adder :: Unsigned 8 -> Unsigned 8 -> Unsigned 8
adder a b = a + b
```
````

Then it runs:

```sh
clash <generated-module> --verilog -main-is adder -outputdir <artifact-dir>/verilog
```

`-main-is` tells Clash to synthesize the binding named by `topEntity=...`
directly. The source passed to Clash is the concatenated user source with
doctests removed; mdbook-clash does not wrap or otherwise modify it. The build
fails if Clash exits unsuccessfully.

## Netlist SVGs

The development shell also includes `netlistsvg`. Add `netlistsvg` to a
synthesized block to inject a rendered netlist SVG:

````md
```haskell,clash group=adder topEntity=adder netlistsvg
adder :: Unsigned 8 -> Unsigned 8 -> Unsigned 8
adder a b = a + b
```
````

The plugin runs Yosys internally to export JSON, then runs
`netlistsvg <json> -o <svg>`. It reads Clash's generated manifest to select the
actual HDL top component, including names set by `Synthesize` annotations. The
default flow then runs `proc`, `opt`, `clean -purge`, and writes JSON.

Add `yosys="..."` to customize the Yosys flow used for the netlist diagram:

````md
```haskell,clash group=adder topEntity=adder yosys="proc; opt; techmap" netlistsvg
adder :: Unsigned 8 -> Unsigned 8 -> Unsigned 8
adder a b = a + b
```
````

The plugin still adds Verilog loading, top-component selection, `clean -purge`,
and `write_json`. The `yosys="..."` commands are inserted in between.
`yosys="..."` without `netlistsvg` is rejected.

The generated Yosys script, JSON, and SVG are kept in the artifact directory;
only the SVG is included in the final documentation.

## Combined simulation and synthesis

````md
```haskell,clash group=increment hidden
module IncrementExample where

import Clash.Prelude
```

```haskell,clash group=increment topEntity=increment
increment :: Unsigned 8 -> Unsigned 8
increment x = x + 1

>>> increment 21
22
```
````

For both simulation and synthesis, the preprocessor separates the doctest
document from the definitions before passing the concatenated user module to
Clash.

## Running the example book

From `mdbook-clash/`:

```sh
nix develop -c mdbook build example
```

The example uses the Clash command supplied by the development shell:

```toml
clash-cmd = ["clash"]
```

The example also uses `netlistsvg`; both it and Yosys are included in the
development shell and the wrapped package.

## Tests

The integration tests do not invoke real Clash. They use fake command
executables and exercise the real mdBook preprocessor entry point:

```sh
nix develop -c cargo test
```

These tests verify command invocation, hidden blocks, unwrapped user modules,
doctest document handling, synthesis, combined checks, caching, and failure
diagnostics. A full `mdbook build example` remains the heavier end-to-end
check.

## Source layout

- `lib.rs` implements the mdBook preprocessor boundary.
- `processor.rs` traverses books and chapters.
- `markdown.rs` parses and validates fenced blocks.
- `source.rs` assembles grouped modules and doctest documents.
- `doctest.rs`, `synthesis.rs`, and `netlist.rs` implement the three build
  phases directly.
- `cache.rs` owns cache keys, manifests, and locking.
- `config.rs` reads `book.toml` configuration.

## Current limitations

- A doctest document must be the final section of its fenced block. Continue
  definitions in a later block in the same group.
- Module declarations, language pragmas, and imports are the responsibility of
  the checked source. Simulation recognizes conventional single-line
  `module Name where` declarations; source without one uses Haskell's implicit
  `Main` module.
- The preprocessor executes local commands while building docs. Treat examples
  as trusted source code.
