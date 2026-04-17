::  lib/rag-logic.hoon: RAG manifest verification & settlement
::
::  RAG-specific gates: prompt reconstruction, manifest verification,
::  and note settlement.  Merkle primitives live in vesl-merkle.hoon.
::  Environment-agnostic: accepts raw nouns, no filesystem coupling.
::
/+  *vesl-merkle
/=  *  /common/zeke
::
|%
::  +build-prompt: deterministic prompt reconstruction
::
::  Concatenates query + \n + chunk1.dat + \n + chunk2.dat + ...
::  Newline separator (byte 0xa) ensures collision-resistant
::  reconstruction. Tail-recursive.
::
::  AUDIT 2026-04-17 M-05: belt-and-suspenders size cap. The Rust
::  hull caps JSON body at 4 MB and manifest total at 10 MB, but a
::  direct poke bypasses those guards. Refuse reconstruction once
::  the built prompt exceeds 10 MB — matches the Rust cap — so the
::  Hoon path has its own ceiling and callers can't crash the
::  Nock stack by handing the kernel a giant manifest.
::
++  build-prompt
  |=  [query=@t dats=(list @t)]
  ^-  @t
  =/  built=@t  query
  =/  sep  10
  =/  max-bytes=@  ^~((mul 10.000 1.000))
  |-
  ?~  dats
    built
  =/  nex=@t  `@t`(cat 3 (cat 3 built sep) i.dats)
  ?:  (gth (met 3 nex) max-bytes)
    ~|  %vesl-prompt-too-large
    !!
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
