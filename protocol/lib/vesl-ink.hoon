::  lib/vesl-ink.hoon: the lightest tentacle
::
::  Commit data, get a root. That's it.
::  tip5 Merkle commitment primitives — no verification, no settlement,
::  no kernel, no state. Just math.
::
::  This is the Ink layer: the minimum viable integration for any
::  NockApp that wants to bind data to a Merkle tree and walk away.
::  If you need to verify or settle, import vesl-logic or vesl-graft.
::
/=  *  /common/zeke
::
|%
::  +split-to-belts: split atom into 7-byte LE chunks
::
::  Each chunk is < 2^56 < Goldilocks prime, ensuring valid tip5
::  field elements for arbitrary-size atoms (cords, hashes, etc.).
::  Cross-VM deterministic: Rust mirrors via bytes.chunks(7).
::
++  split-to-belts
  |=  a=@
  ^-  (list @)
  ?:  =(a 0)  ~[0]
  =/  belts=(list @)  ~
  |-
  ?:  =(a 0)  (flop belts)
  =/  chunk  (end [3 7] a)
  $(a (rsh [3 7] a), belts [chunk belts])
::
::  +hash-leaf: tip5 hash of raw leaf data
::
::  Splits atom into 7-byte field-element chunks, prepends count,
::  hashes via tip5 varlen sponge.  Returns flat atom via
::  digest-to-atom for type compatibility with existing @ fields.
::
++  hash-leaf
  |=  dat=@
  ^-  @
  =/  belts=(list @)  (split-to-belts dat)
  =/  n=@  (lent belts)
  (digest-to-atom:tip5 (hash-belts-list:tip5 [n belts]))
::
::  +hash-pair: tip5 pair hash of two digest atoms
::
::  Converts each flat atom back to a 5-limb noun-digest,
::  hashes the 10 limbs via hash-ten-cell (tip5 fixed sponge).
::
++  hash-pair
  |=  [l=@ r=@]
  ^-  @
  =/  ld=noun-digest:tip5  (atom-to-digest:tip5 l)
  =/  rd=noun-digest:tip5  (atom-to-digest:tip5 r)
  (digest-to-atom:tip5 (hash-ten-cell:tip5 [ld rd]))
--
