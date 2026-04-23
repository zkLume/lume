::  graft-scaffold — starter kernel with Vesl graft pre-wired
::
::  Everything you need to build a grafted NockApp:
::    - settle-graft + vesl-merkle already imported
::    - settle-state composed into versioned-state
::    - all three %settle-* poke delegations written
::    - settle-peek fallthrough in ++peek
::    - one placeholder domain poke (%my-action) to customize
::
::  CUSTOMIZE: rename %my-action, add your state fields, fill in
::  your domain poke body. The graft wiring is done.
::
::  compile: hoonc --new hoon/app/app.hoon hoon/
::
/+  *settle-graft
/+  *vesl-merkle
/=  *  /common/wrapper
::
=>
|%
::  kernel state — your fields + grafted settle state
::
+$  versioned-state
  $:  %v1
      settle=settle-state
      :: CUSTOMIZE: add your state fields here
      items=(map @ @t)
      item-count=@ud
  ==
::
+$  effect  *
::
+$  cause
  $%  [%my-action data=@t]      :: CUSTOMIZE: rename this tag
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
  ::  +peek: query your state or settle state
  ::
  ++  peek
    |=  =path
    ^-  (unit (unit *))
    ?+  path  (settle-peek settle.state path)
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
