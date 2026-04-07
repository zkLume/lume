::  lib/vesl-sigil.hoon: the lightest tier
::
::  Commit data, get a root. That's it.
::  tip5 Merkle commitment primitives — no verification, no settlement,
::  no kernel, no state. Just math.
::
::  This is the Sigil layer: the minimum viable integration for any
::  NockApp that wants to bind data to a Merkle tree and walk away.
::  If you need to verify or settle, import vesl-logic or vesl-graft.
::
::  Re-exports split-to-belts, hash-leaf, hash-pair, verify-chunk
::  from vesl-merkle.
::
/+  *vesl-merkle
|%
--
