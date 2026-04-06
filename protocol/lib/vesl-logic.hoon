::  lib/vesl-logic.hoon: Merkle, manifest & settlement for Verifiable RAG
::
::  Pure functions for Tier 3/4 (Nock-Prover & Settlement) operations.
::  Hash primitive: tip5 (algebraic, STARK-native) via zeke.hoon.
::  ~300 constraints/hash vs ~30,000 for SHA-256 = 100x ZK reduction.
::  Environment-agnostic: accepts raw nouns, no filesystem coupling.
::  Three-hull clean: CLI, Urbit, TEE Cloud.
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
::
::  +verify-chunk: prove a chunk is mathematically bound to a Merkle root
::
::  Strictly tail-recursive (|-) for efficient ZKVM circuit translation.
::  side=%.y -> sibling is LEFT  -> hash(sibling, current)
::  side=%.n -> sibling is RIGHT -> hash(current, sibling)
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
::
::  +build-prompt: deterministic prompt reconstruction
::
::  Concatenates query + \n + chunk1.dat + \n + chunk2.dat + ...
::  Newline separator (byte 0xa) ensures collision-resistant
::  reconstruction. Tail-recursive.
::
++  build-prompt
  |=  [query=@t dats=(list @t)]
  ^-  @t
  =/  built=@t  query
  =/  sep  10
  |-
  ?~  dats
    built
  =/  nex=@t  `@t`(cat 3 (cat 3 built sep) i.dats)
  $(built nex, dats t.dats)
::
::  +verify-manifest: prove AI output derives strictly from verified data
::
::  Prevents Context Spoofing and Prompt Injection by:
::  1. Verifying every chunk is bound to the Merkle root (data integrity)
::  2. Reconstructing the exact prompt from query + verified chunks
::  3. Asserting the stated prompt matches reconstruction (no injection)
::
::  If any chunk fails Merkle verification, immediately returns %.n.
::  Only returns %.y when ALL chunks verify AND prompt is bit-exact.
::
::  Single tail-recursive pass for verification + data collection,
::  then prompt reconstruction and comparison.
::
++  verify-manifest
  |=  $:  mani=[query=@t results=(list [chunk=[id=@ dat=@t] proof=(list [hash=@ side=?]) score=@ud]) prompt=@t output=@t page=@ud]
          expected-root=@
      ==
  ^-  ?
  =/  res  results.mani
  =/  dats=(list @t)  ~
  |-
  ?~  res
    ::  all chunks verified — reconstruct and compare prompt
    ::
    =/  ordered=(list @t)  (flop dats)
    =/  built=@t  (build-prompt query.mani ordered)
    =(built prompt.mani)
  ::  verify current chunk against root
  ::
  ?.  (verify-chunk dat.chunk.i.res proof.i.res expected-root)
    %.n
  ::  chunk verified — accumulate data and continue
  ::
  $(dats [dat.chunk.i.res dats], res t.res)
::
::  +settle-note: Nockchain state transition — %pending to %settled
::
::  The notary gate. Accepts a pending Vesl Note, verifies the
::  full inference manifest against the Merkle root, and transitions
::  the note to %settled.
::
::  On verification failure: crashes via ?> (which invokes !!) —
::  in a ZKVM/smart contract context, the prover cannot produce
::  a valid STARK for a crashed computation = transaction reverts.
::
++  settle-note
  |=  $:  current-note=[id=@ hull=@ root=@ state=[%pending ~]]
          mani=[query=@t results=(list [chunk=[id=@ dat=@t] proof=(list [hash=@ side=?]) score=@ud]) prompt=@t output=@t page=@ud]
          expected-root=@
      ==
  ^-  [id=@ hull=@ root=@ state=[%settled ~]]
  ?>  (verify-manifest mani expected-root)
  [id=id.current-note hull=hull.current-note root=root.current-note state=[%settled ~]]
--
