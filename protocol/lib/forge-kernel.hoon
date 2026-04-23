::  forge-kernel.hoon: STARK tier — generic Merkle verification + proving
::
::  NockApp kernel for domain-agnostic STARK proof generation.
::  Verifies leaf-level Merkle inclusion proofs, then proves correct
::  Nock execution via the STARK prover.  No domain types.  Ship the JAM,
::  developer never touches Hoon.
::
::  Poke causes:
::    [%register hull=@ root=@]  — register a hull's Merkle root
::    [%settle payload=@]         — verify leaves + settle note
::    [%verify payload=@]         — verify leaves (read-only)
::    [%prove payload=@]          — verify + STARK proof + settle
::
::  Compiled: hoonc --new protocol/lib/forge-kernel.hoon hoon/
::  Output:   assets/forge.jam
::
/+  *vesl-merkle
/+  *vesl-prover
/=  *  /common/wrapper
::
=>
|%
+$  versioned-state
  $:  %v1
      registered=(map @ @)
      settled=(set @)
  ==
+$  effect  *
+$  forge-payload
  $:  note=[id=@ hull=@ root=@ state=[%pending ~]]
      leaves=(list [dat=@ proof=(list [hash=@ side=?])])
      expected-root=@
  ==
+$  cause
  $%  [%register hull=@ root=@]
      [%settle payload=@]
      [%verify payload=@]
      [%prove payload=@]
  ==
--
|%
++  moat  (keep versioned-state)
::
++  inner
  |_  state=versioned-state
  ::
  ++  load
    |=  old-state=versioned-state
    ^-  _state
    old-state
  ::
  ++  peek
    |=  =path
    ^-  (unit (unit *))
    ?+  path  ~
      [%registered hull=@ ~]
        =/  vid  +<.path
        ``(~(has by registered.state) vid)
      ::
      [%settled note-id=@ ~]
        =/  nid  +<.path
        ``(~(has in settled.state) nid)
      ::
      [%root hull=@ ~]
        =/  vid  +<.path
        ``(~(get by registered.state) vid)
    ==
  ::
  ++  poke
    |=  =ovum:moat
    ^-  [(list effect) _state]
    =/  act  ((soft cause) cause.input.ovum)
    ?~  act
      ~>  %slog.[3 'forge: invalid cause']
      [~ state]
    ?-  -.u.act
      ::
      ::  %register — store hull root
      ::
        %register
      ?:  (~(has by registered.state) hull.u.act)
        ~>  %slog.[3 'forge: hull already registered']
        [~ state]
      =/  new-reg  (~(put by registered.state) hull.u.act root.u.act)
      :_  state(registered new-reg)
      ^-  (list effect)
      ~[[%registered hull.u.act root.u.act]]
      ::
      ::  %settle — verify all leaves and transition note to %settled
      ::    Guards: root registered, root match, no replay
      ::
        %settle
      =/  raw=*  (cue payload.u.act)
      =/  args=forge-payload  ;;(forge-payload raw)
      ::  Guard: reject unregistered roots
      ::
      ?.  (~(has by registered.state) hull.note.args)
        ~>  %slog.[3 'forge: root not registered']
        [~ state]
      ::  Guard: expected root must match registered root
      ::
      ?.  =(expected-root.args (~(got by registered.state) hull.note.args))
        ~>  %slog.[3 'forge: root mismatch']
        [~ state]
      ::  Guard: note header root must match expected root (H-07)
      ::
      ?.  =(root.note.args expected-root.args)
        ~>  %slog.[3 'forge: note root does not match expected root']
        [~ state]
      ::  Guard: reject duplicate note IDs (replay protection)
      ::
      ?:  (~(has in settled.state) id.note.args)
        ~>  %slog.[3 'forge: note already settled (replay rejected)']
        [~ state]
      ::  Verify all leaves — crash on first failure
      ::
      ?>
        =/  lvs  leaves.args
        |-
        ?~  lvs  %.y
        ?.  (verify-chunk dat.i.lvs proof.i.lvs expected-root.args)  %.n
        $(lvs t.lvs)
      ::  All leaves verified — settle
      ::
      =/  new-settled  (~(put in settled.state) id.note.args)
      :_  state(settled new-settled)
      ^-  (list effect)
      ~[[id.note.args hull.note.args root.note.args [%settled ~]]]
      ::
      ::  %verify — verify leaves (read-only, no state change)
      ::
        %verify
      =/  raw=*  (cue payload.u.act)
      =/  args=forge-payload  ;;(forge-payload raw)
      ?.  (~(has by registered.state) hull.note.args)
        :_  state
        ^-  (list effect)
        ~[[%verified %.n]]
      ?.  =(expected-root.args (~(got by registered.state) hull.note.args))
        :_  state
        ^-  (list effect)
        ~[[%verified %.n]]
      ::  Guard: note header root must match expected root (H-07)
      ::
      ?.  =(root.note.args expected-root.args)
        :_  state
        ^-  (list effect)
        ~[[%verified %.n]]
      =/  ok=?
        =/  lvs  leaves.args
        |-
        ?~  lvs  %.y
        ?.  (verify-chunk dat.i.lvs proof.i.lvs expected-root.args)  %.n
        $(lvs t.lvs)
      :_  state
      ^-  (list effect)
      ~[[%verified ok]]
      ::
      ::  %prove — verify leaves + generate STARK proof (atomic)
      ::    Guards: same as %settle
      ::    If proving crashes, nothing settles.
      ::
        %prove
      =/  raw=*  (cue payload.u.act)
      =/  args=forge-payload  ;;(forge-payload raw)
      ::  Guard: reject unregistered roots
      ::
      ?.  (~(has by registered.state) hull.note.args)
        ~>  %slog.[3 'forge: root not registered']
        [~ state]
      ::  Guard: expected root must match registered root
      ::
      ?.  =(expected-root.args (~(got by registered.state) hull.note.args))
        ~>  %slog.[3 'forge: root mismatch']
        [~ state]
      ::  Guard: note header root must match expected root (H-07)
      ::
      ?.  =(root.note.args expected-root.args)
        ~>  %slog.[3 'forge: note root does not match expected root']
        [~ state]
      ::  Guard: reject duplicate note IDs (replay protection)
      ::
      ?:  (~(has in settled.state) id.note.args)
        ~>  %slog.[3 'forge: note already settled (replay rejected)']
        [~ state]
      ::  Verify all leaves — crash on first failure
      ::
      ?>
        =/  lvs  leaves.args
        |-
        ?~  lvs  %.y
        ?.  (verify-chunk dat.i.lvs proof.i.lvs expected-root.args)  %.n
        $(lvs t.lvs)
      ::  Belt-decompose all leaf data
      ::
      =/  all-belts=(list @)
        =|  acc=(list @)
        =/  lvs  leaves.args
        |-
        ?~  lvs  (flop acc)
        $(acc (weld (flop (split-to-belts dat.i.lvs)) acc), lvs t.lvs)
      ::  Fold all belts to single atom < Goldilocks prime
      ::  p = 2^64 - 2^32 + 1
      ::
      =/  p=@  (add (sub (bex 64) (bex 32)) 1)
      =/  belt-digest=@
        %+  roll  all-belts
        |=  [a=@ b=@]
        (mod (add a b) p)
      ::  64 nested increments on [0 1]
      ::  known-working pattern: atom subject + Nock 0/4 only
      ::
      =/  fs-formula=*
        =/  f=*  [0 1]
        =|  i=@
        |-
        ?:  =(i 64)  f
        $(f [4 f], i +(i))
      =/  result-note  [id.note.args hull.note.args root.note.args [%settled ~]]
      =/  proof-attempt
        %-  mule  |.
        (prove-computation belt-digest fs-formula expected-root.args hull.note.args)
      ?.  -.proof-attempt
        ::  Proof FAILED — jam the trace for Rust-side decoding
        ::
        ~>  %slog.[3 'forge: prove-computation crashed']
        :_  state
        ^-  (list effect)
        ~[[%prove-failed (jam p.proof-attempt)]]
      ::  Proof succeeded — settle and return [result-note proof]
      ::
      =/  new-settled  (~(put in settled.state) id.note.args)
      :_  state(settled new-settled)
      ^-  (list effect)
      ~[[result-note p.proof-attempt]]
    ==
  --
--
((moat |) inner)
