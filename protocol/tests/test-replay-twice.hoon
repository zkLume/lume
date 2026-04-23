::  protocol/tests/test-replay-twice.hoon: replay protection regression test
::
::  AUDIT 2026-04-17 L-04: asserts that poking %settle-note twice with
::  the same payload yields %settle-noted the first time and
::  %settle-error the second time (replay guard in the current epoch).
::  Also covers the cross-epoch case via the prior-settled set (H-01):
::  a rotated epoch must still reject a note-id that landed in the
::  previous epoch.
::
/+  *vesl-merkle
/+  *settle-graft
::
::  Simple hash gate — data is the atom, expected-root is its hash.
::
=/  hash-gate=verify-gate
  |=  [note-id=@ data=* expected-root=@]
  ^-  ?
  =/  dat=@  ;;(@ data)
  =(expected-root (hash-leaf dat))
::
=/  leaf  'replay-regression-payload'
=/  root=@  (hash-leaf leaf)
=/  pending  [id=77 hull=3 root=root state=[%pending ~]]
=/  settle-payload=@  (jam [pending leaf root])
::
::  Register hull=3 with the root.
::
=/  st0=settle-state  new-state
=/  regres  (settle-poke st0 [%settle-register hull=3 root=root] hash-gate)
=/  st1  +.regres
::
::  First settle succeeds.
::
=/  first  (settle-poke st1 [%settle-note payload=settle-payload] hash-gate)
=/  first-efx  -.first
=/  st2  +.first
?>  ?=(^ first-efx)
?>  ?=(%settle-noted -.i.first-efx)
?>  =(77 id.note.i.first-efx)
::
::  Second settle (same payload) is rejected as replay — current epoch.
::
=/  second  (settle-poke st2 [%settle-note payload=settle-payload] hash-gate)
=/  second-efx  -.second
?>  ?=(^ second-efx)
?>  ?=(%settle-error -.i.second-efx)
::
::  Construct the post-rotation state directly and confirm the ID is
::  still blocked via prior-settled. (Auto-rotation fires only at
::  epoch-cap=1M settles, impractical for a unit test. The manual
::  %settle-rotate-epoch arm that used to force rotation here was
::  removed as the AUDIT 2026-04-19 C-01 remediation — two
::  consecutive rotate pokes would have emptied prior-settled.)
::
=/  st3
  %=  st2
    epoch          +(epoch.st2)
    prior-settled  settled.st2
    settled        *(set @)
    settle-count   0
  ==
::
::  state invariants after rotation: epoch bumped, settled empty,
::  prior-settled retains the original note-id.
::
?>  =(+(epoch.st2) epoch.st3)
?>  =(0 ~(wyt in settled.st3))
?>  (~(has in prior-settled.st3) 77)
::
::  Third settle of the same note is rejected from prior-settled.
::
=/  third  (settle-poke st3 [%settle-note payload=settle-payload] hash-gate)
=/  third-efx  -.third
?>  ?=(^ third-efx)
?>  ?=(%settle-error -.i.third-efx)
::
%pass
