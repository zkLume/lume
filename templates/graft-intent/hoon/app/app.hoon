::  graft-intent — NockApp with custom (non-RAG) verification gate
::
::  Proves the Graft works without RAG types. No sur/vesl.hoon,
::  no vesl-logic.hoon. The verification gate is a simple
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
/+  *vesl-graft
/+  *vesl-merkle
/=  *  /common/wrapper
::
=>
|%
::  kernel state — intents + grafted Vesl state
::
+$  versioned-state
  $:  %v1
      vesl=vesl-state
      intents=(map @ @t)
      intent-count=@ud
  ==
::
+$  effect  *
::
+$  cause
  $%  [%declare intent=@t]
      vesl-cause
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
  ::  +peek: query intents or Vesl state
  ::
  ++  peek
    |=  =path
    ^-  (unit (unit *))
    ?+  path  (vesl-peek vesl.state path)
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
        %vesl-register
      =/  lc=vesl-cause  [%vesl-register hull.u.act root.u.act]
      =/  hash-gate=verify-gate
        |=  [data=* expected-root=@]
        ^-  ?
        =((hash-leaf ;;(@ data)) expected-root)
      =/  [efx=(list vesl-effect) new-vesl=vesl-state]
        (vesl-poke vesl.state lc hash-gate)
      :_  state(vesl new-vesl)
      ^-  (list effect)
      efx
      ::
        %vesl-verify
      =/  lc=vesl-cause  [%vesl-verify payload.u.act]
      =/  hash-gate=verify-gate
        |=  [data=* expected-root=@]
        ^-  ?
        =((hash-leaf ;;(@ data)) expected-root)
      =/  [efx=(list vesl-effect) new-vesl=vesl-state]
        (vesl-poke vesl.state lc hash-gate)
      :_  state(vesl new-vesl)
      ^-  (list effect)
      efx
      ::
        %vesl-settle
      =/  lc=vesl-cause  [%vesl-settle payload.u.act]
      =/  hash-gate=verify-gate
        |=  [data=* expected-root=@]
        ^-  ?
        =((hash-leaf ;;(@ data)) expected-root)
      =/  [efx=(list vesl-effect) new-vesl=vesl-state]
        (vesl-poke vesl.state lc hash-gate)
      :_  state(vesl new-vesl)
      ^-  (list effect)
      efx
    ==
  --
--
((moat |) inner)
