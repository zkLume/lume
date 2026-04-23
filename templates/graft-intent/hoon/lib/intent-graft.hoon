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
::  This file is bundled into the graft-intent template for packaging;
::  the canonical source lives at protocol/lib/intent-graft.hoon in the
::  vesl repo.
::
|%
+$  intent-record
  $:  id=@
      hull=@
      body=*
      status=$?(%open %matched %cancelled %expired)
      expires-at=@da
  ==
::
+$  intent-state
  $:  intents=(map @ intent-record)
      intent-count=@ud
      by-hull=(jug @ @)
  ==
::
++  new-state
  ^-  intent-state
  :*  intents=*(map @ intent-record)
      intent-count=0
      by-hull=*(jug @ @)
  ==
::
+$  intent-effect
  $%  [%intent-declared id=@ hull=@]
      [%intent-matched id=@]
      [%intent-cancelled id=@]
      [%intent-expired id=@]
      [%intent-error msg=@t]
  ==
::
+$  intent-cause
  $%  [%intent-declare hull=@ body=* expires-at=@da]
      [%intent-match id=@ proof=*]
      [%intent-cancel id=@]
      [%intent-expire id=@]
  ==
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
++  intent-peek
  |=  [state=intent-state =path]
  ^-  (unit (unit *))
  ~
--
