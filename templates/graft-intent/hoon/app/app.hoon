::  graft-intent — family-5 PLACEHOLDER template
::
::  Composes the crashing intent-graft library into a minimal NockApp
::  kernel. Exists to prove the reservation is real: any %intent-*
::  poke crashes with %intent-graft-placeholder. Do not copy this
::  template for production work — it is a signpost, not a pattern.
::
::  When the Nockchain monorepo publishes a canonical intent structure,
::  the intent-graft library gets swapped for the real primitive and
::  this template becomes a working coordination app by default.
::
::  Compile (from vesl/ root): hoonc templates/graft-intent/hoon/app/app.hoon hoon/ --new
::
/+  *intent-graft
/=  *  /common/wrapper
::
=>
|%
+$  versioned-state
  $:  %v1
      intent=intent-state
  ==
::
+$  effect  *
::
+$  cause  intent-cause
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
  ::
  ++  peek
    |=  =path
    ^-  (unit (unit *))
    (intent-peek intent.state path)
  ::
  ++  poke
    |=  =ovum:moat
    ^-  [(list effect) _state]
    =/  act  ((soft cause) cause.input.ovum)
    ?~  act
      ~>  %slog.[3 'graft-intent: invalid cause']
      [~ state]
    ::  Every cause delegates to intent-poke, and every arm there bangs
    ::  with %intent-graft-placeholder. That crash is the point.
    ::
    =/  [efx=(list intent-effect) new-intent=intent-state]
      (intent-poke intent.state u.act)
    :_  state(intent new-intent)
    ^-  (list effect)
    efx
  --
--
((moat |) inner)
