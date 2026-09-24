# mdbook-clash example with surfer integration

## Signals

Below I added the imports and module decleration, which you cannot see :)

```haskell,clash group=shocking hidden
module Reproducer where

import Clash.Prelude hiding(writeFile, dumpVCD, traceSignal)
import Clash.Shockwaves
import Data.Text.IO  (writeFile)
```

Now for the actual code, here it is!

```haskell,clash group=shocking
constant_signal :: Signal System (Maybe (Unsigned 12))
constant_signal = pure $ Just 1005

incrementing :: HiddenClockResetEnable dom => Signal dom (Unsigned 6)
incrementing = register 0 (incrementing + 1)

decrementing :: HiddenClockResetEnable dom => Signal dom (Unsigned 6)
decrementing = register 0 (register 0 (decrementing - 1))
```

I wonder how this looks like in an image though... Hmmm..
Oh hey look at that!

```haskell,clash group=shocking shockwaves=0,10,incrementing,decrementing,constant_signal hidden
```

