::  graft-settle — NockApp with full Vesl settlement tier
::
::  Extends the graft-mint pattern with settlement: verify data
::  integrity AND transition notes from %pending to %settled.
::  Replay protection included.
::
::  Domain: a report submission system.  Users submit reports,
::  the system commits them to a Merkle tree, and settlements
::  create a permanent verifiable record.
::
::  Demonstrates:
::    - full Graft with settlement (%settle-note)
::    - replay protection (can't settle the same note twice)
::    - domain state alongside verification + settlement state
::    - the Beak pattern: commit → register → settle
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
::  kernel state — reports + grafted settle state
::
+$  versioned-state
  $:  %v1
      settle=settle-state
      reports=(map @ @t)
      report-count=@ud
  ==
::
+$  effect  *
::
+$  cause
  $%  [%submit title=@t body=@t]
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
  ::  +peek: query reports or settle state
  ::
  ++  peek
    |=  =path
    ^-  (unit (unit *))
    ?+  path  (settle-peek settle.state path)
      [%report id=@ ~]
        =/  rid  +<.path
        ``(~(get by reports.state) rid)
      ::
      [%count ~]
        ``report-count.state
    ==
  ::  +poke: submit reports or delegate to Graft
  ::
  ++  poke
    |=  =ovum:moat
    ^-  [(list effect) _state]
    =/  act  ((soft cause) cause.input.ovum)
    ?~  act
      ~>  %slog.[3 'graft-settle: invalid cause']
      [~ state]
    ?-  -.u.act
      ::  domain: submit a report
      ::    stores report under incrementing ID, emits the ID
      ::
        %submit
      =/  rid  report-count.state
      =/  content=@t  (cat 3 title.u.act (cat 3 10 body.u.act))
      =/  new-reports  (~(put by reports.state) rid content)
      ~>  %slog.[0 (cat 3 'report #' (scot %ud rid))]
      :_  state(reports new-reports, report-count +(rid))
      ^-  (list effect)
      ~[[%submitted rid content]]
      ::
      ::  --- grafted tentacle (settlement) ---
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
      ::  %settle-note — the Beak.  verify + state transition.
      ::  on success, the note is permanently settled.
      ::  on failure (bad proof, unregistered root, replay),
      ::  the Graft returns an error effect.
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
