::  settle-kernel.hoon: heavy tier — full settlement, no STARK
::
::  NockApp kernel for Merkle root registration, manifest verification,
::  and note settlement.  Everything vesl-kernel.hoon does, minus the
::  STARK prover and tx-engine.  Settlement is where soft state becomes
::  hard record.
::
::  Why no tx-engine: sig-hash/tx-id computation pulled in tx-engine-0
::  (71K lines, 135s compile) making the JAR 18MB — same as forge.
::  The Rust hull handles transaction building natively.  Settle stays
::  focused: verify data, settle notes, done.
::
::  Poke causes:
::    [%register hull=@ root=@]  — register a hull's Merkle root
::    [%settle payload=@]          — verify manifest + settle note
::    [%verify payload=@]          — verify manifest (read-only)
::
::  Compiled: hoonc --new protocol/lib/settle-kernel.hoon hoon/
::  Output:   assets/settle.jam
::
/-  *vesl
/+  *rag-logic
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
+$  cause
  $%  [%register hull=@ root=@]
      [%settle payload=@]
      [%verify payload=@]
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
      ~>  %slog.[3 'settle: invalid cause']
      [~ state]
    ?-  -.u.act
      ::
      ::  %register — store hull root
      ::
        %register
      ::  Guard: reject re-registration (hull already has a root)
      ::
      ?:  (~(has by registered.state) hull.u.act)
        ~>  %slog.[3 'settle: hull already registered']
        [~ state]
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
        ~>  %slog.[3 'settle: root not registered']
        [~ state]
      ::  Guard: expected root must match registered root
      ::
      ?.  =(expected-root.args (~(got by registered.state) hull.note.args))
        ~>  %slog.[3 'settle: root mismatch']
        [~ state]
      ::  Guard: note header root must match expected root (H-07)
      ::
      ?.  =(root.note.args expected-root.args)
        ~>  %slog.[3 'settle: note root does not match expected root']
        [~ state]
      ::  Guard: reject duplicate note IDs (replay protection)
      ::
      ?:  (~(has in settled.state) id.note.args)
        ~>  %slog.[3 'settle: note already settled (replay rejected)']
        [~ state]
      =/  result  (settle-note note.args mani.args expected-root.args)
      =/  new-settled  (~(put in settled.state) id.note.args)
      :_  state(settled new-settled)
      ^-  (list effect)
      ~[result]
      ::
      ::  %verify — verify manifest (read-only, no state change)
      ::
        %verify
      =/  raw=*  (cue payload.u.act)
      =/  args=settlement-payload  ;;(settlement-payload raw)
      ?.  (~(has by registered.state) hull.note.args)
        :_  state
        ^-  (list effect)
        ~[[%verified %.n]]
      ::  Guard: expected root must match registered root
      ::
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
      =/  ok=?  (verify-manifest mani.args expected-root.args)
      :_  state
      ^-  (list effect)
      ~[[%verified ok]]
    ==
  --
--
((moat |) inner)
