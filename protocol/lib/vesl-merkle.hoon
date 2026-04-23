::  lib/vesl-merkle.hoon: generic Merkle primitives for Vesl
::
::  Pure math — no RAG types, no domain logic.
::  Hash primitive: tip5 (algebraic, STARK-native) via zeke.hoon.
::  Import this for Merkle commitment, leaf hashing, and proof
::  verification in any verification gate.
::
::  Shared-merkle surface — the following arms are the stable
::  primitives every graft (mint, guard, settle, forge) reuses.
::  Do not rename or reshape without coordinating across all
::  four primitives; the graft-inject manifest imports block
::  pastes `/+  *vesl-merkle` into composed kernels under the
::  assumption that these arms exist by these names.
::
::    hash-leaf       tip5 hash of raw atom data.  Used by mint
::                    for commitment, by guard for leaf check,
::                    by settle's default verify-gate, by forge
::                    for Fiat-Shamir leaf binding.
::    hash-pair       tip5 pair hash of two digest atoms.  Used
::                    inside verify-chunk and by any graft that
::                    folds up an internal Merkle node.
::    verify-chunk    prove a chunk is bound to a Merkle root via
::                    a sibling-hash proof.  Depth-capped at 64.
::    split-to-belts  atom -> 7-byte field-element list.  Used by
::                    forge's belt-digest fold and by any gate that
::                    wants to hash cell-shaped data as a flat atom.
::    belts-to-atom   inverse of split-to-belts.  Used by Rust-side
::                    reconstruction paths that consume belt lists.
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
::  +belts-to-atom: reconstruct atom from 7-byte LE belt list
::
::  Inverse of split-to-belts.  Concatenates each belt as a
::  7-byte little-endian block via rep.
::
++  belts-to-atom
  |=  belts=(list @)
  ^-  @
  (rep [3 7] belts)
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
::
::  +verify-chunk: prove a chunk is mathematically bound to a Merkle root
::
::  Strictly tail-recursive (|-) for efficient ZKVM circuit translation.
::  side=%.y -> sibling is LEFT  -> hash(sibling, current)
::  side=%.n -> sibling is RIGHT -> hash(current, sibling)
::  Max proof depth: 64 nodes (supports 2^64 leaves).
::
++  verify-chunk
  |=  [chunk=@ proof=(list [hash=@ side=?]) expected-root=@]
  ^-  ?
  ?:  (gth (lent proof) 64)  %.n
  =/  cur=@  (hash-leaf chunk)
  |-
  ?~  proof
    =(cur expected-root)
  =/  nex=@
    ?:  side.i.proof
      (hash-pair hash.i.proof cur)
    (hash-pair cur hash.i.proof)
  $(cur nex, proof t.proof)
--
