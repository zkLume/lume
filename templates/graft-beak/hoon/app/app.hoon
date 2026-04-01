::  graft-beak — NockApp with full Vesl settlement tentacle
::
::  Extends the graft-ink pattern with settlement: verify data
::  integrity AND transition notes from %pending to %settled.
::  Replay protection included.  The only hard part of an octopus.
::
::  Domain: a report submission system.  Users submit reports,
::  the system commits them to a Merkle tree, and settlements
::  create a permanent verifiable record.
::
::  Demonstrates:
::    - full Graft with settlement (%vesl-settle)
::    - replay protection (can't settle the same note twice)
::    - domain state alongside verification + settlement state
::    - the Beak pattern: commit → register → settle
::
::  Compile: hoonc hoon/app/app.hoon $NOCK_HOME/hoon/
::
/-  *vesl
/+  *vesl-graft
/=  *  /common/wrapper
::
=>
|%
::  kernel state — reports + grafted Vesl state
::
+$  versioned-state
  $:  %v1
      vesl=vesl-state
      reports=(map @ @t)
      report-count=@ud
  ==
::
+$  effect  *
::
+$  cause
  $%  [%submit title=@t body=@t]
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
  ::  +peek: query reports or Vesl state
  ::
  ++  peek
    |=  =path
    ^-  (unit (unit *))
    ?+  path  (vesl-peek vesl.state path)
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
      ~>  %slog.[3 'graft-beak: invalid cause']
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
      ::
        %vesl-register
      =/  lc=vesl-cause  [%vesl-register hull.u.act root.u.act]
      =/  [efx=(list vesl-effect) new-vesl=vesl-state]
        (vesl-poke vesl.state lc)
      :_  state(vesl new-vesl)
      ^-  (list effect)
      efx
      ::
        %vesl-verify
      =/  lc=vesl-cause  [%vesl-verify payload.u.act]
      =/  [efx=(list vesl-effect) new-vesl=vesl-state]
        (vesl-poke vesl.state lc)
      :_  state(vesl new-vesl)
      ^-  (list effect)
      efx
      ::
      ::  %vesl-settle — the Beak.  verify + state transition.
      ::  on success, the note is permanently settled.
      ::  on failure (bad proof, unregistered root, replay),
      ::  the Graft returns an error effect.
      ::
        %vesl-settle
      =/  lc=vesl-cause  [%vesl-settle payload.u.act]
      =/  [efx=(list vesl-effect) new-vesl=vesl-state]
        (vesl-poke vesl.state lc)
      :_  state(vesl new-vesl)
      ^-  (list effect)
      efx
    ==
  --
--
((moat |) inner)
