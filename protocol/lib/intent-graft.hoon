::  lib/intent-graft.hoon — FAMILY 5 PLACEHOLDER (crashes on invocation)
::
::  Reserves the family-5 intent-coordination slot in vesl's 5-family
::  graft catalog. Every cause arm bangs with %intent-graft-placeholder.
::  Do not build production logic against this library — the Nockchain
::  monorepo has not yet published a canonical intent structure, so the
::  shape here is a best-guess design sketch that will be swapped when
::  upstream publishes.
::
::  A quiet %intent-error effect would invite callers to paper over the
::  placeholder with a retry loop. Crashing forces anyone composing the
::  graft to know they are building on an unfinished primitive.
::
::  See:
::    .dev/BIFURCATE_INTENT.md — the design sketch this placeholder draws from
::    .dev/GRAFT_REFACTOR.md  — the 5-family catalog plan that reserves this slot
::    docs/graft-manifest.md  — the authoritative priority lattice
::
|%
::  +$intent-record: placeholder shape. Fields may change when the real
::    primitive lands.
::
+$  intent-record
  $:  id=@
      hull=@
      body=*
      status=$?(%open %matched %cancelled %expired)
      expires-at=@da
  ==
::
::  +$intent-state: state fragment grafted onto the host kernel.
::    Kept structurally sound so the kernel's ++load and peek arms
::    compile cleanly around the graft; callers still can't exercise
::    the poke arms without crashing.
::
+$  intent-state
  $:  intents=(map @ intent-record)
      intent-count=@ud
      by-hull=(jug @ @)
  ==
::
::  +new-state: fresh empty intent state. Safe to instantiate even
::    though the poke arms crash — the ability to initialize an empty
::    state is a property of the placeholder, not an invitation to use it.
::
++  new-state
  ^-  intent-state
  :*  intents=*(map @ intent-record)
      intent-count=0
      by-hull=*(jug @ @)
  ==
::
::  +$intent-effect: effect union (placeholder — shape may change).
::
+$  intent-effect
  $%  [%intent-declared id=@ hull=@]
      [%intent-matched id=@]
      [%intent-cancelled id=@]
      [%intent-expired id=@]
      [%intent-error msg=@t]
  ==
::
::  +$intent-cause: tagged pokes the real primitive will handle.
::    Declaring the full shape here reserves the cause-tag namespace
::    so downstream grafts don't collide with family 5 while we wait
::    for upstream.
::
+$  intent-cause
  $%  [%intent-declare hull=@ body=* expires-at=@da]
      [%intent-match id=@ proof=*]
      [%intent-cancel id=@]
      [%intent-expire id=@]
  ==
::
::  +intent-poke: placeholder dispatcher. Every arm bangs with
::    %intent-graft-placeholder. The ?- switch is kept explicit so
::    the cause-tag surface is self-documenting even though no arm
::    does any work.
::
++  intent-poke
  |=  [state=intent-state cause=intent-cause]
  ^-  [(list intent-effect) intent-state]
  ?-  -.cause
      %intent-declare
    ~|  %intent-graft-placeholder
    !!
      %intent-match
    ~|  %intent-graft-placeholder
    !!
      %intent-cancel
    ~|  %intent-graft-placeholder
    !!
      %intent-expire
    ~|  %intent-graft-placeholder
    !!
  ==
::
::  +intent-peek: placeholder query arm. Returns ~ for every path so
::    the host kernel's peek chain can fall through to the next graft
::    (per the graft-inject peek-chain convention). When the real
::    primitive lands this arm will grow `[%intent ...]` paths.
::
++  intent-peek
  |=  [state=intent-state =path]
  ^-  (unit (unit *))
  ~
--
