::  protocol/lib/vesl-kernel.hoon: NockApp kernel for settlement + proving
::
::  Proper NockApp kernel with versioned state, poke, peek, and load arms.
::  Poke causes:
::    [%register hull=@ root=@]  — register a hull root
::    [%settle payload=@]          — verify manifest + settle note
::    [%prove  payload=@]          — settle + generate STARK proof
::    [%sig-hash seeds-jam=@ fee=@] — compute sig-hash from seeds + fee
::    [%tx-id spends-jam=@]        — compute tx-id from spends
::
::  The settle/prove payload is a jammed settlement-payload atom.
::  The sig-hash/tx-id pokes use Hoon's tx-engine hashable infrastructure
::  to produce byte-exact hashes for Rust-assembled transactions.
::
::  Security hardening:
::    - %settle/%prove reject unregistered roots (must %register first)
::    - %settle/%prove reject duplicate note IDs (replay protection)
::
::  Compiled: hoonc --new protocol/lib/vesl-kernel.hoon hoon/
::
/-  *vesl
/+  *vesl-logic
/+  *vesl-prover
/=  *  /common/wrapper
/=  txv1  /common/tx-engine-1
::
=>
|%
::  Kernel state — tracks registered roots and settled notes
::
+$  versioned-state
  $:  %v1
      registered=(map @ @)
      settled=(set @)
  ==
::  Effects the kernel can produce
::
+$  effect  *
::  Causes the kernel accepts
::
+$  cause
  $%  [%register hull=@ root=@]
      [%settle payload=@]
      [%prove payload=@]
      [%sig-hash seeds-jam=@ fee=@]
      [%tx-id spends-jam=@]
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
    ==
  ::
  ++  poke
    |=  =ovum:moat
    ^-  [(list effect) _state]
    =/  act  ((soft cause) cause.input.ovum)
    ?~  act
      ~>  %slog.[3 'vesl: invalid cause']
      [~ state]
    ?-  -.u.act
      ::
      ::  %register — store hull root, return confirmation
      ::
        %register
      =/  new-reg  (~(put by registered.state) hull.u.act root.u.act)
      :_  state(registered new-reg)
      ^-  (list effect)
      ~[[%registered hull.u.act root.u.act]]
      ::
      ::  %settle — verify manifest and transition note to %settled
      ::    Guards: root must be registered, note ID must not be settled
      ::
        %settle
      =/  raw=*  (cue payload.u.act)
      =/  args=settlement-payload  ;;(settlement-payload raw)
      ::  Guard: reject unregistered roots
      ::
      ?.  (~(has by registered.state) hull.note.args)
        ~>  %slog.[3 'vesl: root not registered']
        [~ state]
      ::  Guard: reject duplicate note IDs (replay protection)
      ::
      ?:  (~(has in settled.state) id.note.args)
        ~>  %slog.[3 'vesl: note already settled (replay rejected)']
        [~ state]
      =/  result  (settle-note note.args mani.args expected-root.args)
      =/  new-settled  (~(put in settled.state) id.note.args)
      :_  state(settled new-settled)
      ^-  (list effect)
      ~[result]
      ::
      ::  %prove — settle + generate STARK proof (atomic)
      ::    Guards: same as %settle
      ::    If proving crashes, nothing settles. Use %settle for
      ::    settlement without proof.
      ::
        %prove
      =/  raw=*  (cue payload.u.act)
      =/  args=settlement-payload  ;;(settlement-payload raw)
      ::  Guard: reject unregistered roots
      ::
      ?.  (~(has by registered.state) hull.note.args)
        ~>  %slog.[3 'vesl: root not registered']
        [~ state]
      ::  Guard: reject duplicate note IDs (replay protection)
      ::
      ?:  (~(has in settled.state) id.note.args)
        ~>  %slog.[3 'vesl: note already settled (replay rejected)']
        [~ state]
      ::  Verify manifest (must pass before we attempt proving)
      ::
      =/  result-note  (settle-note note.args mani.args expected-root.args)
      ::  STARK proof of note commitment: proves [note-id hull-id root]
      ::  was the subject of a Nock computation.  The settle-note gate
      ::  already verified the full manifest; this proof binds the
      ::  settlement metadata to a cryptographic attestation.
      ::
      ::  We prove *[[id hull root] [0 1]] — the identity function
      ::  on a small tuple.  The STARK's public inputs contain the
      ::  note-id, hull-id, and merkle-root, tying the proof to
      ::  this specific settlement.
      ::
      ::  mule catches stack overflows and prover crashes -- the prover
      ::  needs ~3GB and will crash on default 1GB stacks.
      ::
      =/  commitment  [id.note.args hull.note.args expected-root.args]
      =/  proof-attempt  (mule |.((prove-computation commitment [0 1])))
      ?.  -.proof-attempt
        ::  Proof FAILED -- do NOT settle, return error effect
        ::
        ~>  %slog.[3 'vesl: prove-computation crashed']
        :_  state
        ^-  (list effect)
        ~[[%prove-failed ~]]
      ::  Proof succeeded -- settle and return [result-note proof]
      ::
      =/  new-settled  (~(put in settled.state) id.note.args)
      :_  state(settled new-settled)
      ^-  (list effect)
      ~[[result-note p.proof-attempt]]
      ::
      ::  %sig-hash — compute sig-hash from jammed seeds + fee
      ::    Uses tx-engine's hashable infrastructure for byte-exact hashes.
      ::    Stateless: does not modify kernel state.
      ::
        %sig-hash
      =/  sds=seeds:txv1  ;;(seeds:txv1 (cue seeds-jam.u.act))
      =/  result=hash:txv1
        %-  hash-hashable:tip5
        [(sig-hashable:seeds:txv1 sds) leaf+fee.u.act]
      :_  state
      ^-  (list effect)
      ~[[%sig-hash result]]
      ::
      ::  %tx-id — compute tx-id from jammed spends
      ::    Uses tx-engine's hashable infrastructure for byte-exact hashes.
      ::    Stateless: does not modify kernel state.
      ::
        %tx-id
      =/  sps=spends:txv1  ;;(spends:txv1 (cue spends-jam.u.act))
      =/  result=tx-id:txv1
        %-  hash-hashable:tip5
        [leaf+%1 (hashable:spends:txv1 sps)]
      :_  state
      ^-  (list effect)
      ~[[%tx-id result]]
    ==
  --
--
((moat |) inner)
