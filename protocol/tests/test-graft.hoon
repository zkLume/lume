::  protocol/tests/test-graft.hoon: vesl-graft composable state test
::
::  Tests the Graft lifecycle: register → peek → settle → replay guard.
::  Uses the hedge fund scenario from test-entrypoint.hoon.
::  Compilation success = all assertions passed.
::
/-  *vesl
/+  *vesl-logic
/+  *vesl-graft
::
::  ============================================
::  SETUP: Build valid hedge fund scenario
::  ============================================
::
=/  h0  (hash-leaf 'Q3 revenue: $47M, up 12% YoY')
=/  h1  (hash-leaf 'Risk exposure: 15% in emerging markets')
=/  h2  (hash-leaf 'Board approved new derivatives desk')
=/  h3  (hash-leaf 'Compliance review scheduled for Oct')
=/  h01  (hash-pair h0 h1)
=/  h23  (hash-pair h2 h3)
=/  root  (hash-pair h01 h23)
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
  [query=query results=results prompt=valid-prompt output='Based on your Q3 data...']
::
=/  pending-note  [id=42 hull=7 root=root state=[%pending ~]]
::
::  ============================================
::  TEST 1: Fresh state is empty
::  ============================================
::
=/  st=vesl-state  [registered=*(map @ @) settled=*(set @)]
::
::  Peek for unregistered hull -> %.n
::
=/  peek-unreg  (vesl-peek st /registered/7)
?>  ?=(^ peek-unreg)
?>  ?=(^ u.peek-unreg)
?>  =(%.n ;;(? u.u.peek-unreg))
::
::  ============================================
::  TEST 2: Register a hull root
::  ============================================
::
=/  reg-result  (vesl-poke st [%vesl-register hull=7 root=root])
=/  reg-effects  -.reg-result
=/  st  +.reg-result
::
::  Effect should be [%vesl-registered 7 root]
::
?>  ?=(^ reg-effects)
?>  ?=(%vesl-registered -.i.reg-effects)
?>  =(7 hull.i.reg-effects)
?>  =(root root.i.reg-effects)
::
::  Peek should now return %.y
::
=/  peek-reg  (vesl-peek st /registered/7)
?>  ?=(^ peek-reg)
?>  ?=(^ u.peek-reg)
?>  =(%.y ;;(? u.u.peek-reg))
::
::  Root peek should return the root
::
=/  peek-root  (vesl-peek st /root/7)
?>  ?=(^ peek-root)
?>  ?=(^ u.peek-root)
?>  =(`root ;;((unit @) u.u.peek-root))
::
::  ============================================
::  TEST 3: Verify a manifest (pure, no state change)
::  ============================================
::
=/  verify-payload=@  (jam [pending-note valid-mani root])
=/  ver-result  (vesl-poke st [%vesl-verify payload=verify-payload])
=/  ver-effects  -.ver-result
=/  st  +.ver-result
::
?>  ?=(^ ver-effects)
?>  ?=(%vesl-verified -.i.ver-effects)
?>  =(%.y ok.i.ver-effects)
::
::  State should be unchanged (no settlement)
::
=/  peek-not-settled  (vesl-peek st /settled/42)
?>  ?=(^ peek-not-settled)
?>  ?=(^ u.peek-not-settled)
?>  =(%.n ;;(? u.u.peek-not-settled))
::
::  ============================================
::  TEST 4: Settle a note
::  ============================================
::
=/  settle-payload=@  (jam [pending-note valid-mani root])
=/  set-result  (vesl-poke st [%vesl-settle payload=settle-payload])
=/  set-effects  -.set-result
=/  st  +.set-result
::
::  Effect should be %vesl-settled with the settled note
::
?>  ?=(^ set-effects)
?>  ?=(%vesl-settled -.i.set-effects)
?>  =(42 id.note.i.set-effects)
?>  =(7 hull.note.i.set-effects)
?>  =([%settled ~] state.note.i.set-effects)
::
::  Peek: note 42 should be settled
::
=/  peek-settled  (vesl-peek st /settled/42)
?>  ?=(^ peek-settled)
?>  ?=(^ u.peek-settled)
?>  =(%.y ;;(? u.u.peek-settled))
::
::  ============================================
::  TEST 5: Replay protection — re-settling same note fails
::  ============================================
::
=/  replay-result  (vesl-poke st [%vesl-settle payload=settle-payload])
=/  replay-effects  -.replay-result
::
::  Should get a %vesl-error, not a %vesl-settled
::
?>  ?=(^ replay-effects)
?>  ?=(%vesl-error -.i.replay-effects)
::
::  ============================================
::  TEST 6: Unregistered root settlement fails
::  ============================================
::
::  Create a note pointing to a hull that isn't registered
::
=/  bad-note  [id=99 hull=999 root=root state=[%pending ~]]
=/  bad-payload=@  (jam [bad-note valid-mani root])
=/  unreg-result  (vesl-poke st [%vesl-settle payload=bad-payload])
=/  unreg-effects  -.unreg-result
::
?>  ?=(^ unreg-effects)
?>  ?=(%vesl-error -.i.unreg-effects)
::
%pass
