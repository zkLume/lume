::  graft-scaffold — starter kernel with Vesl graft pre-wired
::
::  Everything you need to build a grafted NockApp:
::    - vesl-graft + vesl-merkle already imported
::    - vesl-state composed into versioned-state
::    - all three %vesl-* poke delegations written
::    - vesl-peek fallthrough in ++peek
::    - one placeholder domain poke (%my-action) to customize
::
::  CUSTOMIZE: rename %my-action, add your state fields, fill in
::  your domain poke body. The graft wiring is done.
::
::  compile: hoonc --new hoon/app/app.hoon hoon/
::
/+  *vesl-graft
/+  *vesl-merkle
/=  *  /common/wrapper
::
=>
|%
::  kernel state — your fields + grafted vesl state
::
+$  versioned-state
  $:  %v1
      vesl=vesl-state
      :: CUSTOMIZE: add your state fields here
      items=(map @ @t)
      item-count=@ud
  ==
::
+$  effect  *
::
+$  cause
  $%  [%my-action data=@t]      :: CUSTOMIZE: rename this tag
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
  ::  +peek: query your state or vesl state
  ::
  ++  peek
    |=  =path
    ^-  (unit (unit *))
    ?+  path  (vesl-peek vesl.state path)
      :: CUSTOMIZE: add your peek paths
      [%item id=@ ~]
        =/  iid  +<.path
        ``(~(get by items.state) iid)
      ::
      [%count ~]
        ``item-count.state
    ==
  ::  +poke: handle domain actions or delegate to graft
  ::
  ++  poke
    |=  =ovum:moat
    ^-  [(list effect) _state]
    =/  act  ((soft cause) cause.input.ovum)
    ?~  act
      ~>  %slog.[3 'graft-scaffold: invalid cause']
      [~ state]
    ?-  -.u.act
      ::  CUSTOMIZE: your domain poke
      ::
        %my-action
      =/  iid  item-count.state
      =/  new-items  (~(put by items.state) iid data.u.act)
      ~>  %slog.[0 (cat 3 'item #' (scot %ud iid))]
      :_  state(items new-items, item-count +(iid))
      ^-  (list effect)
      ~[[%my-actioned iid data.u.act]]
      ::
      ::  --- grafted verification (hash gate) ---
      ::  default gate: tip5-hash the data, compare to root.
      ::  replace with your own verify-gate for domain logic.
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
