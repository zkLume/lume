::  graft-intent — NockApp with custom (non-RAG) verification gate
::
::  Proves the Graft works without RAG types. No sur/vesl.hoon,
::  no rag-logic.hoon. The verification gate is a simple
::  hash-comparison: hash the data, compare to expected root.
::
::  Domain: an intent registry. Users declare intents (strings),
::  the system commits them to a Merkle tree, and settlement
::  proves an intent was committed before it was executed.
::
::  The custom gate:
::    |=  [data=* expected-root=@]
::    =((hash-leaf ;;(@ data)) expected-root)
::
::  This is the simplest possible verify-gate: "the tip5 hash
::  of this data equals the expected root." One line. No manifest,
::  no proofs, no retrieval scores. Just math.
::
::  Compile: hoonc hoon/app/app.hoon $NOCK_HOME/hoon/
::
/+  *settle-graft
/+  *vesl-merkle
/=  *  /common/wrapper
::
=>
|%
::  kernel state — intents + grafted settle state
::
+$  versioned-state
  $:  %v1
      settle=settle-state
      intents=(map @ @t)
      intent-count=@ud
  ==
::
+$  effect  *
::
+$  cause
  $%  [%declare intent=@t]
      settle-cause
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
  ::  +peek: query intents or settle state
  ::
  ++  peek
    |=  =path
    ^-  (unit (unit *))
    ?+  path  (settle-peek settle.state path)
      [%intent id=@ ~]
        =/  iid  +<.path
        ``(~(get by intents.state) iid)
      ::
      [%count ~]
        ``intent-count.state
    ==
  ::  +poke: declare intents or delegate to Graft
  ::
  ++  poke
    |=  =ovum:moat
    ^-  [(list effect) _state]
    =/  act  ((soft cause) cause.input.ovum)
    ?~  act
      ~>  %slog.[3 'graft-intent: invalid cause']
      [~ state]
    ?-  -.u.act
      ::  domain: declare an intent
      ::
        %declare
      =/  iid  intent-count.state
      =/  new-intents  (~(put by intents.state) iid intent.u.act)
      ~>  %slog.[0 (cat 3 'intent #' (scot %ud iid))]
      :_  state(intents new-intents, intent-count +(iid))
      ^-  (list effect)
      ~[[%declared iid intent.u.act]]
      ::
      ::  --- grafted verification (custom gate) ---
      ::  the hash gate: tip5-hash the data, compare to root.
      ::  no manifest, no proofs, no RAG types.
      ::
        %settle-register
      =/  lc=settle-cause  [%settle-register hull.u.act root.u.act]
      =/  hash-gate=verify-gate
        |=  [note-id=@ data=* expected-root=@]
        ^-  ?
        =((hash-leaf ;;(@ data)) expected-root)
      =/  [efx=(list settle-effect) new-settle=settle-state]
        (settle-poke settle.state lc hash-gate)
      :_  state(settle new-settle)
      ^-  (list effect)
      efx
      ::
        %settle-verify
      =/  lc=settle-cause  [%settle-verify payload.u.act]
      =/  hash-gate=verify-gate
        |=  [note-id=@ data=* expected-root=@]
        ^-  ?
        =((hash-leaf ;;(@ data)) expected-root)
      =/  [efx=(list settle-effect) new-settle=settle-state]
        (settle-poke settle.state lc hash-gate)
      :_  state(settle new-settle)
      ^-  (list effect)
      efx
      ::
        %settle-note
      =/  lc=settle-cause  [%settle-note payload.u.act]
      =/  hash-gate=verify-gate
        |=  [note-id=@ data=* expected-root=@]
        ^-  ?
        =((hash-leaf ;;(@ data)) expected-root)
      =/  [efx=(list settle-effect) new-settle=settle-state]
        (settle-poke settle.state lc hash-gate)
      :_  state(settle new-settle)
      ^-  (list effect)
      efx
    ==
  --
--
((moat |) inner)
