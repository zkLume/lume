::  graft-mint — NockApp with Vesl settle tier
::
::  A note store with Merkle commitment verification grafted on.
::  Your domain logic (%put, %del) lives next to Vesl verification
::  (%settle-register, %settle-verify) in the same kernel.  Zero
::  verification code written — the Graft handles it.
::
::  This is the pattern: compose settle-state into your state,
::  delegate tagged pokes to settle-poke, done.
::
::  Demonstrates:
::    - composing settle-state into versioned-state
::    - delegating %settle-* pokes to the Graft
::    - delegating /settle-registered and /settle-root peeks to the Graft
::    - domain logic alongside verification logic
::
::  Compile: hoonc hoon/app/app.hoon $NOCK_HOME/hoon/
::
/-  *vesl
/+  *settle-graft
/+  *rag-logic
/=  *  /common/wrapper
::
=>
|%
::  kernel state — domain state + grafted settle state
::
+$  versioned-state
  $:  %v1
      settle=settle-state
      notes=(map @t @t)
  ==
::
+$  effect  *
::
+$  cause
  $%  [%put key=@t val=@t]
      [%del key=@t]
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
  ::  +peek: query domain state or settle state
  ::    domain peeks: /note/<key>, /count
  ::    graft peeks:  /settle-registered/<hull>, /settle-root/<hull>
  ::
  ++  peek
    |=  =path
    ^-  (unit (unit *))
    ?+  path  (settle-peek settle.state path)
      [%note key=@t ~]
        =/  k  +<.path
        ``(~(get by notes.state) k)
      ::
      [%count ~]
        ``~(wyt by notes.state)
    ==
  ::  +poke: handle domain mutations and settle pokes
  ::
  ++  poke
    |=  =ovum:moat
    ^-  [(list effect) _state]
    =/  act  ((soft cause) cause.input.ovum)
    ?~  act
      ~>  %slog.[3 'graft-mint: invalid cause']
      [~ state]
    ?-  -.u.act
      ::  domain: store a note
      ::
        %put
      =/  new-notes  (~(put by notes.state) key.u.act val.u.act)
      ~>  %slog.[0 (cat 3 'note: ' key.u.act)]
      :_  state(notes new-notes)
      ^-  (list effect)
      ~[[%put key.u.act val.u.act]]
      ::  domain: delete a note
      ::
        %del
      =/  new-notes  (~(del by notes.state) key.u.act)
      ~>  %slog.[0 (cat 3 'deleted: ' key.u.act)]
      :_  state(notes new-notes)
      ^-  (list effect)
      ~[[%deleted key.u.act]]
      ::
      ::  --- grafted verification ---
      ::  everything below is delegation.  settle-poke handles
      ::  the verification logic, we just wire state in and out.
      ::  the RAG gate casts opaque data to a manifest and verifies it.
      ::
        %settle-register
      =/  lc=settle-cause  [%settle-register hull.u.act root.u.act]
      =/  rag-gate=verify-gate
        |=  [note-id=@ data=* expected-root=@]
        ^-  ?
        =/  mani  ;;(manifest data)
        (verify-manifest mani expected-root)
      =/  [efx=(list settle-effect) new-settle=settle-state]
        (settle-poke settle.state lc rag-gate)
      :_  state(settle new-settle)
      ^-  (list effect)
      efx
      ::
        %settle-verify
      =/  lc=settle-cause  [%settle-verify payload.u.act]
      =/  rag-gate=verify-gate
        |=  [note-id=@ data=* expected-root=@]
        ^-  ?
        =/  mani  ;;(manifest data)
        (verify-manifest mani expected-root)
      =/  [efx=(list settle-effect) new-settle=settle-state]
        (settle-poke settle.state lc rag-gate)
      :_  state(settle new-settle)
      ^-  (list effect)
      efx
      ::
        %settle-note
      =/  lc=settle-cause  [%settle-note payload.u.act]
      =/  rag-gate=verify-gate
        |=  [note-id=@ data=* expected-root=@]
        ^-  ?
        =/  mani  ;;(manifest data)
        (verify-manifest mani expected-root)
      =/  [efx=(list settle-effect) new-settle=settle-state]
        (settle-poke settle.state lc rag-gate)
      :_  state(settle new-settle)
      ^-  (list effect)
      efx
    ==
  --
--
((moat |) inner)
