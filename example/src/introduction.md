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
