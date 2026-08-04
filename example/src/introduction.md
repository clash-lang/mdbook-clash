# mdbook-clash example

This block is simulated during `mdbook build`:

```haskell,clash
double :: Unsigned 8 -> Unsigned 8
double x = x + x

>>> double 10
20
```

This block is synthesized to Verilog during `mdbook build` because it has a
`topEntity` attribute:

```haskell,clash topEntity=adder yosys="proc;" netlistsvg
adder :: Unsigned 8 -> Unsigned 8 -> Unsigned 8
adder a b = a + b
```

```haskell,clash topEntity=increment
increment :: Unsigned 8 -> Unsigned 8
increment x = x + 1

x = 10 + 11

>>> increment x
22
```

## Grouped blocks with multiple synthesis targets

Blocks with the same `group` identifier are combined into one set of
definitions. This first block provides a definition shared by both synthesis
targets below:

```haskell,clash group=adjusters
adjustment :: Unsigned 8
adjustment = 1
```

This block is simulated independently and synthesized as `addAdjustment`:

```haskell,clash group=adjusters topEntity=addAdjustment
addAdjustment :: Unsigned 8 -> Unsigned 8
addAdjustment value = value + adjustment

>>> addAdjustment 41
42
```

This block belongs to the same group, so its simulation and synthesis can use
both `adjustment` and `addAdjustment`. It is synthesized separately as
`twiceAdjusted`:

```haskell,clash group=adjusters topEntity=twiceAdjusted
twiceAdjusted :: Unsigned 8 -> Unsigned 8
twiceAdjusted value = addAdjustment (addAdjustment value)

>>> twiceAdjusted 40
43
```

The grouped definitions are compiled once for simulation, while the two
`topEntity` attributes produce two independent Clash synthesis runs.
