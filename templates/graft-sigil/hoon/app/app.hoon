::  graft-sigil — NockApp with Vesl Sigil + Vigil tiers
::
::  A note store with Merkle commitment verification grafted on.
::  Your domain logic (%put, %del) lives next to Vesl verification
::  (%vesl-register, %vesl-verify) in the same kernel.  Zero
::  verification code written — the Graft handles it.
::
::  This is the pattern: compose vesl-state into your state,
::  delegate tagged pokes to vesl-poke, done.
::
::  Demonstrates:
::    - composing vesl-state into versioned-state
::    - delegating %vesl-* pokes to the Graft
::    - delegating /registered and /root peeks to the Graft
::    - domain logic alongside verification logic
::
::  Compile: hoonc hoon/app/app.hoon $NOCK_HOME/hoon/
::
/-  *vesl
/+  *vesl-graft
/+  *vesl-logic
/=  *  /common/wrapper
::
=>
|%
::  kernel state — domain state + grafted Vesl state
::
+$  versioned-state
  $:  %v1
      vesl=vesl-state
      notes=(map @t @t)
  ==
::
+$  effect  *
::
+$  cause
  $%  [%put key=@t val=@t]
      [%del key=@t]
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
  ::  +peek: query domain state or Vesl state
  ::    domain peeks: /note/<key>, /count
  ::    graft peeks:  /registered/<hull>, /root/<hull>
  ::
  ++  peek
    |=  =path
    ^-  (unit (unit *))
    ?+  path  (vesl-peek vesl.state path)
      [%note key=@t ~]
        =/  k  +<.path
        ``(~(get by notes.state) k)
      ::
      [%count ~]
        ``~(wyt by notes.state)
    ==
  ::  +poke: handle domain mutations and Vesl pokes
  ::
  ++  poke
    |=  =ovum:moat
    ^-  [(list effect) _state]
    =/  act  ((soft cause) cause.input.ovum)
    ?~  act
      ~>  %slog.[3 'graft-sigil: invalid cause']
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
      ::  everything below is delegation.  vesl-poke handles
      ::  the verification logic, we just wire state in and out.
      ::  the RAG gate casts opaque data to a manifest and verifies it.
      ::
        %vesl-register
      =/  lc=vesl-cause  [%vesl-register hull.u.act root.u.act]
      =/  rag-gate=verify-gate
        |=  [data=* expected-root=@]
        ^-  ?
        =/  mani  ;;(manifest data)
        (verify-manifest mani expected-root)
      =/  [efx=(list vesl-effect) new-vesl=vesl-state]
        (vesl-poke vesl.state lc rag-gate)
      :_  state(vesl new-vesl)
      ^-  (list effect)
      efx
      ::
        %vesl-verify
      =/  lc=vesl-cause  [%vesl-verify payload.u.act]
      =/  rag-gate=verify-gate
        |=  [data=* expected-root=@]
        ^-  ?
        =/  mani  ;;(manifest data)
        (verify-manifest mani expected-root)
      =/  [efx=(list vesl-effect) new-vesl=vesl-state]
        (vesl-poke vesl.state lc rag-gate)
      :_  state(vesl new-vesl)
      ^-  (list effect)
      efx
      ::
        %vesl-settle
      =/  lc=vesl-cause  [%vesl-settle payload.u.act]
      =/  rag-gate=verify-gate
        |=  [data=* expected-root=@]
        ^-  ?
        =/  mani  ;;(manifest data)
        (verify-manifest mani expected-root)
      =/  [efx=(list vesl-effect) new-vesl=vesl-state]
        (vesl-poke vesl.state lc rag-gate)
      :_  state(vesl new-vesl)
      ^-  (list effect)
      efx
    ==
  --
--
((moat |) inner)
