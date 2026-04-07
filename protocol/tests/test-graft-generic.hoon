::  protocol/tests/test-graft-generic.hoon: gate-agnostic Graft test
::
::  Tests the generic Graft with a non-RAG verification gate.
::  No /-  *vesl, no /+  *vesl-logic — proves the Graft works
::  for any computation type, not just RAG manifests.
::  Compilation success = all assertions passed.
::
/+  *vesl-merkle
/+  *vesl-graft
::
::  ============================================
::  SETUP: Simple hash-comparison gate
::  ============================================
::
::  This gate verifies that hash-leaf(data) == expected-root.
::  The simplest possible verification: one leaf, one root.
::  data is cast to @ (bare atom), hashed, compared.
::
=/  hash-gate=verify-gate
  |=  [data=* expected-root=@]
  ^-  ?
  =/  dat=@  ;;(@ data)
  =(expected-root (hash-leaf dat))
::
::  Build test data: a single leaf whose root is its own hash
::
=/  leaf-data=@  'test-payload-alpha'
=/  root=@  (hash-leaf leaf-data)
::
::  ============================================
::  TEST 1: Fresh state is empty
::  ============================================
::
=/  st=vesl-state  [registered=*(map @ @) settled=*(set @)]
::
=/  peek-unreg  (vesl-peek st /registered/7)
?>  ?=(^ peek-unreg)
?>  ?=(^ u.peek-unreg)
?>  =(%.n ;;(? u.u.peek-unreg))
::
::  ============================================
::  TEST 2: Register hull root
::  ============================================
::
=/  reg-result  (vesl-poke st [%vesl-register hull=7 root=root] hash-gate)
=/  reg-effects  -.reg-result
=/  st  +.reg-result
::
?>  ?=(^ reg-effects)
?>  ?=(%vesl-registered -.i.reg-effects)
?>  =(7 hull.i.reg-effects)
?>  =(root root.i.reg-effects)
::
::  Peek confirms registration
::
=/  peek-reg  (vesl-peek st /registered/7)
?>  ?=(^ peek-reg)
?>  ?=(^ u.peek-reg)
?>  =(%.y ;;(? u.u.peek-reg))
::
::  ============================================
::  TEST 3: Settle with passing hash gate
::  ============================================
::
::  Build graft-payload: note + data (the raw leaf) + expected-root
::
=/  pending-note  [id=1 hull=7 root=root state=[%pending ~]]
=/  settle-payload=@  (jam [pending-note leaf-data root])
=/  set-result  (vesl-poke st [%vesl-settle payload=settle-payload] hash-gate)
=/  set-effects  -.set-result
=/  st  +.set-result
::
?>  ?=(^ set-effects)
?>  ?=(%vesl-settled -.i.set-effects)
?>  =(1 id.note.i.set-effects)
?>  =(7 hull.note.i.set-effects)
?>  =([%settled ~] state.note.i.set-effects)
::
::  Peek confirms settlement
::
=/  peek-settled  (vesl-peek st /settled/1)
?>  ?=(^ peek-settled)
?>  ?=(^ u.peek-settled)
?>  =(%.y ;;(? u.u.peek-settled))
::
::  ============================================
::  TEST 4: Replay protection
::  ============================================
::
=/  replay-result  (vesl-poke st [%vesl-settle payload=settle-payload] hash-gate)
=/  replay-effects  -.replay-result
::
?>  ?=(^ replay-effects)
?>  ?=(%vesl-error -.i.replay-effects)
::
::  ============================================
::  TEST 5: Unregistered root rejection
::  ============================================
::
=/  bad-note  [id=2 hull=999 root=root state=[%pending ~]]
=/  bad-payload=@  (jam [bad-note leaf-data root])
=/  unreg-result  (vesl-poke st [%vesl-settle payload=bad-payload] hash-gate)
=/  unreg-effects  -.unreg-result
::
?>  ?=(^ unreg-effects)
?>  ?=(%vesl-error -.i.unreg-effects)
::
::  ============================================
::  TEST 6: Root mismatch rejection
::  ============================================
::
=/  wrong-root=@  (hash-leaf 'different-data')
=/  mismatch-note  [id=3 hull=7 root=root state=[%pending ~]]
=/  mismatch-payload=@  (jam [mismatch-note leaf-data wrong-root])
=/  mismatch-result  (vesl-poke st [%vesl-settle payload=mismatch-payload] hash-gate)
=/  mismatch-effects  -.mismatch-result
::
?>  ?=(^ mismatch-effects)
?>  ?=(%vesl-error -.i.mismatch-effects)
::
::  ============================================
::  TEST 7: Registration overwrite protection
::  ============================================
::
=/  other-root=@  (hash-leaf 'other-leaf')
=/  overwrite-result  (vesl-poke st [%vesl-register hull=7 root=other-root] hash-gate)
=/  overwrite-effects  -.overwrite-result
::
?>  ?=(^ overwrite-effects)
?>  ?=(%vesl-error -.i.overwrite-effects)
::
::  ============================================
::  TEST 8: Verify with passing gate (read-only)
::  ============================================
::
=/  ver-note  [id=10 hull=7 root=root state=[%pending ~]]
=/  ver-payload=@  (jam [ver-note leaf-data root])
=/  ver-result  (vesl-poke st [%vesl-verify payload=ver-payload] hash-gate)
=/  ver-effects  -.ver-result
::
?>  ?=(^ ver-effects)
?>  ?=(%vesl-verified -.i.ver-effects)
?>  =(%.y ok.i.ver-effects)
::
::  ============================================
::  TEST 9: Verify with wrong data (read-only, returns %.n)
::  ============================================
::
::  Use a failing gate that always returns %.n
::
=/  fail-gate=verify-gate
  |=  [data=* expected-root=@]
  ^-  ?
  %.n
::
=/  fail-ver-result  (vesl-poke st [%vesl-verify payload=ver-payload] fail-gate)
=/  fail-ver-effects  -.fail-ver-result
::
?>  ?=(^ fail-ver-effects)
?>  ?=(%vesl-verified -.i.fail-ver-effects)
?>  =(%.n ok.i.fail-ver-effects)
::
%pass
