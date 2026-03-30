::  sur/vesl.hoon: Sovereign-RAG core data structures
::
::  Tier 1-4 types for the Hull architecture.
::  Designed for ZK-circuit translation via Zorp ZKVM.
::  All hash fields are bare @ for minimal prover overhead.
::
|%
::  Tier 1: Sovereign Storage
::
+$  chunk-id  @
+$  chunk  [id=chunk-id dat=@t]
+$  merkle-root  @
+$  proof-node  [hash=@ side=?]
+$  merkle-proof  (list proof-node)
::
::  Tier 2: Local Inference
::
+$  retrieval
  $:  =chunk
      proof=merkle-proof
      score=@ud
  ==
+$  manifest
  $:  query=@t
      results=(list retrieval)
      prompt=@t
      output=@t
  ==
::
::  Tier 3: Nock-Prover
::
+$  nock-zkp
  $:  root=merkle-root
      prf=@
      stamp=@da
  ==
::
::  Tier 4: Settlement
::
+$  hull-id  @
+$  note-state
  $%  [%pending ~]
      [%verified p=nock-zkp]
      [%settled ~]
  ==
+$  note
  $:  id=@
      hull=hull-id
      root=merkle-root
      state=note-state
  ==
::
::  Tier 5: ABI Boundary
::
::  The strict type for cross-VM settlement payloads.
::  Defines the exact noun structure the Rust Hull must produce.
::
+$  settlement-payload
  $:  note=[id=@ hull=@ root=@ state=[%pending ~]]
      mani=[query=@t results=(list [chunk=[id=@ dat=@t] proof=(list [hash=@ side=?]) score=@ud]) prompt=@t output=@t]
      expected-root=@
  ==
--
