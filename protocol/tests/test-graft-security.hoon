::  protocol/tests/test-graft-security.hoon: V-001 remediation tests
::
::  Tests the two security hardening additions:
::    1. Root binding — settlement and verify reject mismatched expected-root
::    2. Registration overwrite protection — re-registering a hull is rejected
::
::  Compilation success = all assertions passed.
::
/-  *vesl
/+  *vesl-merkle
/+  *rag-logic
/+  *settle-graft
::
::  ============================================
::  SETUP: Build two valid Merkle trees
::  ============================================
::
::  Tree A (legitimate)
::
=/  h0  (hash-leaf 'Q3 revenue: $47M, up 12% YoY')
=/  h1  (hash-leaf 'Risk exposure: 15% in emerging markets')
=/  h2  (hash-leaf 'Board approved new derivatives desk')
=/  h3  (hash-leaf 'Compliance review scheduled for Oct')
=/  h01  (hash-pair h0 h1)
=/  h23  (hash-pair h2 h3)
=/  root-a  (hash-pair h01 h23)
::
::  Tree B (attacker's fabricated tree)
::
=/  f0  (hash-leaf 'fabricated data 0')
=/  f1  (hash-leaf 'fabricated data 1')
=/  root-b  (hash-pair f0 f1)
::
::  Build valid manifest for tree A
::
=/  chunk0  [id=0 dat='Q3 revenue: $47M, up 12% YoY']
=/  chunk1  [id=1 dat='Risk exposure: 15% in emerging markets']
=/  proof0=(list [hash=@ side=?])
  ~[[hash=h1 side=%.n] [hash=h23 side=%.n]]
=/  proof1=(list [hash=@ side=?])
  ~[[hash=h0 side=%.y] [hash=h23 side=%.n]]
=/  results=(list [chunk=[id=@ dat=@t] proof=(list [hash=@ side=?]) score=@ud])
  ~[[chunk=chunk0 proof=proof0 score=950.000] [chunk=chunk1 proof=proof1 score=870.000]]
::
=/  query=@t  'Summarize the hedge fund Q3 performance'
=/  sep  10
=/  s1  (cat 3 query sep)
=/  s2  (cat 3 s1 'Q3 revenue: $47M, up 12% YoY')
=/  s3  (cat 3 s2 sep)
=/  valid-prompt=@t  `@t`(cat 3 s3 'Risk exposure: 15% in emerging markets')
=/  valid-mani
  [query=query results=results prompt=valid-prompt output='Based on your Q3 data...' page=0]
::
::  RAG verification gate — wraps verify-manifest for settle-poke
::
=/  rag-gate=verify-gate
  |=  [note-id=@ data=* expected-root=@]
  ^-  ?
  =/  mani  ;;(manifest data)
  (verify-manifest mani expected-root)
::
::  ============================================
::  TEST 1: Registration overwrite protection
::  ============================================
::
::  Register hull=7 with root-a
::
=/  st=settle-state  new-state
=/  reg-result  (settle-poke st [%settle-register hull=7 root=root-a] rag-gate)
=/  st  +.reg-result
::
::  Attempt to re-register hull=7 with root-b — must be rejected
::
=/  overwrite-result  (settle-poke st [%settle-register hull=7 root=root-b] rag-gate)
=/  overwrite-effects  -.overwrite-result
=/  overwrite-state  +.overwrite-result
::
::  Effect must be an error
::
?>  ?=(^ overwrite-effects)
?>  ?=(%settle-error -.i.overwrite-effects)
::
::  State must be unchanged — hull=7 still maps to root-a
::
?>  =(root-a (~(got by registered.overwrite-state) 7))
::
::  ============================================
::  TEST 2: Settlement with mismatched root — must reject
::  ============================================
::
::  Build payload: note points to hull=7 (registered with root-a)
::  but expected-root is root-b (attacker's root)
::
=/  pending-note  [id=42 hull=7 root=root-a state=[%pending ~]]
=/  bad-root-payload=@  (jam [pending-note valid-mani root-b])
=/  mismatch-result  (settle-poke st [%settle-note payload=bad-root-payload] rag-gate)
=/  mismatch-effects  -.mismatch-result
::
::  Must get %settle-error for root mismatch, not a settlement
::
?>  ?=(^ mismatch-effects)
?>  ?=(%settle-error -.i.mismatch-effects)
::
::  ============================================
::  TEST 3: Verify with mismatched root — must reject
::  ============================================
::
=/  ver-mismatch-result  (settle-poke st [%settle-verify payload=bad-root-payload] rag-gate)
=/  ver-mismatch-effects  -.ver-mismatch-result
::
::  Must get %settle-verified %.n (not %.y)
::
?>  ?=(^ ver-mismatch-effects)
?>  ?=(%settle-verified -.i.ver-mismatch-effects)
?>  =(%.n ok.i.ver-mismatch-effects)
::
::  ============================================
::  TEST 4: Valid settlement still works (regression check)
::  ============================================
::
::  Build payload with correct root-a
::
=/  good-payload=@  (jam [pending-note valid-mani root-a])
=/  good-result  (settle-poke st [%settle-note payload=good-payload] rag-gate)
=/  good-effects  -.good-result
=/  st  +.good-result
::
::  Must succeed with %settle-noted
::
?>  ?=(^ good-effects)
?>  ?=(%settle-noted -.i.good-effects)
?>  =(42 id.note.i.good-effects)
?>  =(7 hull.note.i.good-effects)
?>  =([%settled ~] state.note.i.good-effects)
::
::  ============================================
::  TEST 5: Full attack scenario — register, overwrite, settle
::  ============================================
::
::  Fresh state — simulate the full V-001 attack
::
=/  st2=settle-state  new-state
::
::  Step 1: Legitimate registration
::
=/  legit-reg  (settle-poke st2 [%settle-register hull=7 root=root-a] rag-gate)
=/  st2  +.legit-reg
::
::  Step 2: Attacker tries to overwrite — must fail
::
=/  attack-reg  (settle-poke st2 [%settle-register hull=7 root=root-b] rag-gate)
=/  attack-effects  -.attack-reg
?>  ?=(^ attack-effects)
?>  ?=(%settle-error -.i.attack-effects)
::
::  Step 3: Attacker tries to settle with root-b — must fail
::
=/  attack-note  [id=1 hull=7 root=root-b state=[%pending ~]]
=/  attack-payload=@  (jam [attack-note valid-mani root-b])
=/  attack-settle  (settle-poke st2 [%settle-note payload=attack-payload] rag-gate)
=/  settle-effects  -.attack-settle
?>  ?=(^ settle-effects)
?>  ?=(%settle-error -.i.settle-effects)
::
::  Confirm hull=7 root is still root-a
::
?>  =(root-a (~(got by registered.st2) 7))
::
%pass
