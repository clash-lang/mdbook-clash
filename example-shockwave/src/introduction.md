# mdbook-clash example with surfer integration

## Signals

Woah look at this!

```haskell,clash group=shocking hidden
module Reproducer where

import Clash.Prelude hiding(writeFile, dumpVCD, traceSignal)
import Clash.Shockwaves
import Data.Text.IO  (writeFile)
```

Now it may come as a surprise that

```haskell,clash group=shocking shockwaves=0,100,incrementing,decrementing
constant_signal :: Signal System (Unsigned 8)
constant_signal = pure 230

incrementing :: HiddenClockResetEnable dom => Signal dom (Unsigned 6)
incrementing = register 0 (incrementing + 1)

decrementing :: HiddenClockResetEnable dom => Signal dom (Unsigned 6)
decrementing = register 0 (decrementing - 1)
```

Yay!
