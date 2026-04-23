::  guard-kernel.hoon: mid tier — commitment + verification
::
::  NockApp kernel for Merkle root registration, chunk verification,
::  and full manifest verification.  No settlement state transitions,
::  no STARK proofs, no tx-engine.
::
::  Keep guard: verify data integrity without settling.
::
::  Poke causes:
::    [%register hull=@ root=@]  — register a hull's Merkle root
::    [%verify payload=@]          — verify manifest against registered root
::
::  The verify payload is a jammed settlement-payload (reuses the type
::  for compatibility, but only reads the manifest + root fields).
::
::  Compiled: hoonc --new protocol/lib/guard-kernel.hoon hoon/
::  Output:   assets/guard.jam
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
  ==
+$  effect  *
+$  cause
  $%  [%register hull=@ root=@]
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
      ~>  %slog.[3 'guard: invalid cause']
      [~ state]
    ?-  -.u.act
      ::
      ::  %register — store hull root
      ::
        %register
      ::  Guard: reject re-registration (hull already has a root)
      ::
      ?:  (~(has by registered.state) hull.u.act)
        ~>  %slog.[3 'guard: hull already registered']
        [~ state]
      =/  new-reg  (~(put by registered.state) hull.u.act root.u.act)
      :_  state(registered new-reg)
      ^-  (list effect)
      ~[[%registered hull.u.act root.u.act]]
      ::
      ::  %verify — verify manifest against registered root
      ::    Guard: root must be registered
      ::    Returns [%verified ok=?] — no state change
      ::
        %verify
      =/  raw=*  (cue payload.u.act)
      =/  args=settlement-payload  ;;(settlement-payload raw)
      ::  Guard: reject unregistered roots
      ::
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
      ::
      =/  ok=?  (verify-manifest mani.args expected-root.args)
      :_  state
      ^-  (list effect)
      ~[[%verified ok]]
    ==
  --
--
((moat |) inner)
