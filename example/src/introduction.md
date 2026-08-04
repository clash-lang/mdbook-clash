# mdbook-clash example

## Standalone block

An ungrouped block is already a complete listing, so it contains its own module
and imports and does not get a separate full-listing link:

```haskell,clash topEntity=standaloneIncrement
module StandaloneExample where

import Clash.Prelude

standaloneIncrement :: Unsigned 8 -> Unsigned 8
standaloneIncrement value = value + 1

>>> standaloneIncrement 7
8
```

## Grouped blocks

This block is simulated during `mdbook build`:

```haskell,clash group=double-example hidden
module DoubleExample where

import Clash.Prelude
```

```haskell,clash group=double-example
double :: Unsigned 8 -> Unsigned 8
double x = x + x

>>> double 10
20
```

This block is synthesized to Verilog during `mdbook build` because it has a
`topEntity` attribute:

```haskell,clash group=adder-example hidden
module AdderExample where

import Clash.Prelude
```

```haskell,clash group=adder-example topEntity=adder yosys="proc;" netlistsvg
adder :: Unsigned 8 -> Unsigned 8 -> Unsigned 8
adder a b = a + b
```

```haskell,clash group=increment-example hidden
module IncrementExample where

import Clash.Prelude
```

```haskell,clash group=increment-example topEntity=increment
increment :: Unsigned 8 -> Unsigned 8
increment x = x + 1

x :: Unsigned 8
x = 10 + 11

>>> increment x
22
```

## Grouped blocks with multiple synthesis targets

Blocks with the same `group` identifier are combined into one set of
definitions. This first block provides a definition shared by both synthesis
targets below:

```haskell,clash group=adjusters hidden
module Adjusters where
import Clash.Prelude
import qualified Data.List as L
```

```haskell,clash group=adjusters
adjustment :: Unsigned 8
adjustment = 1
```

This block is simulated independently and synthesized as `addAdjustment`:

```haskell,clash group=adjusters topEntity=addAdjustment
addAdjustment :: Unsigned 8 -> Unsigned 8
addAdjustment value = value + adjustment

>>> L.map addAdjustment [40, 41]
[41,42]
```

This block belongs to the same group, so its simulation and synthesis can use
both `adjustment` and `addAdjustment`. It is synthesized separately as
`twiceAdjusted`:

```haskell,clash group=adjusters topEntity=twiceAdjusted
twiceAdjusted :: Unsigned 8 -> Unsigned 8
twiceAdjusted value = addAdjustment (addAdjustment value)

>>> twiceAdjusted 40
42
```

The grouped definitions are loaded into one Clash interpreter, while the two
`topEntity` attributes produce two independent Clash synthesis runs.
