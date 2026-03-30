::  protocol/tests/test-entrypoint.hoon: ABI boundary serialization test
::
::  Tests the jam/cue boundary between Rust Hull and Nock Prover.
::  Proves valid payloads settle and malformed payloads crash at
::  the mold boundary before reaching any logic.
::  Compilation success = all assertions passed.
::
/-  *vesl
/+  *vesl-logic
/+  *vesl-entrypoint
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
::  TEST 1: Valid jammed payload -> settled note
::  ============================================
::
::  Simulate Rust Hull: jam the [note manifest root] tuple
::  into a single atom, exactly as the off-chain client would.
::
=/  payload=@  (jam [pending-note valid-mani root])
::
::  Pass through the ABI boundary
::
=/  result  (vesl-entrypoint payload)
?>  =(state.result [%settled ~])
?>  =(id.result 42)
?>  =(hull.result 7)
::
::  ============================================
::  TEST 2: Garbage payload -> crash at mold
::  ============================================
::
::  Simulate malicious/buggy Rust client sending wrong noun shape.
::  The ;; mold sees an atom where it expects [note mani root]
::  and crashes BEFORE any logic executes.
::
=/  garbage=@  (jam [%malicious 'data'])
=/  crash-test  (mule |.((vesl-entrypoint garbage)))
?<  -.crash-test
::
%pass
