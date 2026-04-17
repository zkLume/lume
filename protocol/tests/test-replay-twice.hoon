::  protocol/tests/test-replay-twice.hoon: replay protection regression test
::
::  AUDIT 2026-04-17 L-04: asserts that poking %vesl-settle twice with
::  the same payload yields %vesl-settled the first time and
::  %vesl-error the second time (replay guard in the current epoch).
::  Also covers the cross-epoch case via the prior-settled set (H-01):
::  a rotated epoch must still reject a note-id that landed in the
::  previous epoch.
::
/+  *vesl-merkle
/+  *vesl-graft
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
=/  st0=vesl-state  new-state
=/  regres  (vesl-poke st0 [%vesl-register hull=3 root=root] hash-gate)
=/  st1  +.regres
::
::  First settle succeeds.
::
=/  first  (vesl-poke st1 [%vesl-settle payload=settle-payload] hash-gate)
=/  first-efx  -.first
=/  st2  +.first
?>  ?=(^ first-efx)
?>  ?=(%vesl-settled -.i.first-efx)
?>  =(77 id.note.i.first-efx)
::
::  Second settle (same payload) is rejected as replay — current epoch.
::
=/  second  (vesl-poke st2 [%vesl-settle payload=settle-payload] hash-gate)
=/  second-efx  -.second
?>  ?=(^ second-efx)
?>  ?=(%vesl-error -.i.second-efx)
::
::  Force rotation and confirm the ID is still blocked via prior-settled.
::
=/  rotres  (vesl-poke st2 [%vesl-rotate-epoch ~] hash-gate)
=/  st3  +.rotres
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
=/  third  (vesl-poke st3 [%vesl-settle payload=settle-payload] hash-gate)
=/  third-efx  -.third
?>  ?=(^ third-efx)
?>  ?=(%vesl-error -.i.third-efx)
::
%pass
