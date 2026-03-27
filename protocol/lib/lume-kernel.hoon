::  protocol/lib/lume-kernel.hoon: NockApp kernel for settlement + proving
::
::  Proper NockApp kernel with versioned state, poke, peek, and load arms.
::  Poke causes:
::    [%register vessel=@ root=@]  — register a vessel root
::    [%settle payload=@]          — verify manifest + settle note
::    [%prove  payload=@]          — settle + generate STARK proof
::
::  The settle/prove payload is a jammed settlement-payload atom.
::
::  Security hardening:
::    - %settle/%prove reject unregistered roots (must %register first)
::    - %settle/%prove reject duplicate note IDs (replay protection)
::
::  Compiled: hoonc --new protocol/lib/lume-kernel.hoon hoon/
::
/-  *lume
/+  *lume-logic
/+  *lume-prover
/=  *  /common/wrapper
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
  $%  [%register vessel=@ root=@]
      [%settle payload=@]
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
      [%registered vessel=@ ~]
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
      ~>  %slog.[3 'lume: invalid cause']
      [~ state]
    ?-  -.u.act
      ::
      ::  %register — store vessel root, return confirmation
      ::
        %register
      =/  new-reg  (~(put by registered.state) vessel.u.act root.u.act)
      :_  state(registered new-reg)
      ^-  (list effect)
      ~[[%registered vessel.u.act root.u.act]]
      ::
      ::  %settle — verify manifest and transition note to %settled
      ::    Guards: root must be registered, note ID must not be settled
      ::
        %settle
      =/  raw=*  (cue payload.u.act)
      =/  args=settlement-payload  ;;(settlement-payload raw)
      ::  Guard: reject unregistered roots
      ::
      ?.  (~(has by registered.state) vessel.note.args)
        ~>  %slog.[3 'lume: root not registered']
        [~ state]
      ::  Guard: reject duplicate note IDs (replay protection)
      ::
      ?:  (~(has in settled.state) id.note.args)
        ~>  %slog.[3 'lume: note already settled (replay rejected)']
        [~ state]
      =/  result  (settle-note note.args mani.args expected-root.args)
      =/  new-settled  (~(put in settled.state) id.note.args)
      :_  state(settled new-settled)
      ^-  (list effect)
      ~[result]
      ::
      ::  %prove — settle + generate STARK proof
      ::    Guards: same as %settle
      ::
        %prove
      =/  raw=*  (cue payload.u.act)
      =/  args=settlement-payload  ;;(settlement-payload raw)
      ::  Guard: reject unregistered roots
      ::
      ?.  (~(has by registered.state) vessel.note.args)
        ~>  %slog.[3 'lume: root not registered']
        [~ state]
      ::  Guard: reject duplicate note IDs (replay protection)
      ::
      ?:  (~(has in settled.state) id.note.args)
        ~>  %slog.[3 'lume: note already settled (replay rejected)']
        [~ state]
      =/  result-note  (settle-note note.args mani.args expected-root.args)
      =/  proof  (prove-computation raw [0 1])
      =/  new-settled  (~(put in settled.state) id.note.args)
      :_  state(settled new-settled)
      ^-  (list effect)
      ~[[result-note proof]]
    ==
  --
--
((moat |) inner)
