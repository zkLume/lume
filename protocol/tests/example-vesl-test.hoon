::  tests/example-vesl-test.hoon: Example using vesl-test library
::
::  Demonstrates all assertion patterns from vesl-test.
::  Compile with: hoonc protocol/tests/example-vesl-test.hoon $NOCK_HOME/hoon/
::  Compilation success = all tests passed.
::
/-  *vesl
/+  *vesl-logic
/+  *vesl-test
::
::  ============================================
::  SETUP: 4-leaf Merkle tree
::  ============================================
::
=/  h0  (hash-leaf 'alpha')
=/  h1  (hash-leaf 'bravo')
=/  h2  (hash-leaf 'charlie')
=/  h3  (hash-leaf 'delta')
=/  h01  (hash-pair h0 h1)
=/  h23  (hash-pair h2 h3)
=/  root  (hash-pair h01 h23)
::
::  ============================================
::  TEST 1: assert-eq — value equality
::  ============================================
::
?>  (assert-eq (hash-leaf 'alpha') h0)
?>  (assert-eq (hash-pair h0 h1) h01)
::
::  ============================================
::  TEST 2: assert-neq — value inequality
::  ============================================
::
?>  (assert-neq h0 h1)
?>  (assert-neq (hash-pair h0 h1) (hash-pair h1 h0))
::
::  ============================================
::  TEST 3: assert-hash-eq / assert-hash-neq
::  ============================================
::
?>  (assert-hash-eq root (hash-pair h01 h23))
?>  (assert-hash-neq h0 h1)
::
::  ============================================
::  TEST 4: assert-flag — boolean gate results
::  ============================================
::
=/  proof0=(list [hash=@ side=?])
  ~[[hash=h1 side=%.n] [hash=h23 side=%.n]]
::
?>  (assert-flag (verify-chunk 'alpha' proof0 root))
?>  (assert-not-flag (verify-chunk 'TAMPERED' proof0 root))
::
::  ============================================
::  TEST 5: assert-crash — expected failures
::  ============================================
::
::  settle-note crashes on invalid manifests
::
=/  chunk0  [id=0 dat='alpha']
=/  chunk1  [id=1 dat='bravo']
=/  proof1=(list [hash=@ side=?])
  ~[[hash=h0 side=%.y] [hash=h23 side=%.n]]
=/  results=(list [chunk=[id=@ dat=@t] proof=(list [hash=@ side=?]) score=@ud])
  ~[[chunk=chunk0 proof=proof0 score=100] [chunk=chunk1 proof=proof1 score=100]]
::
=/  query=@t  'test query'
=/  sep  10
=/  s1  (cat 3 query sep)
=/  s2  (cat 3 s1 'alpha')
=/  s3  (cat 3 s2 sep)
=/  valid-prompt=@t  `@t`(cat 3 s3 'bravo')
=/  valid-mani
  [query=query results=results prompt=valid-prompt output='test output']
=/  pending-note  [id=1 hull=1 root=root state=[%pending ~]]
::
::  Valid case succeeds
?>  (assert-ok |.((settle-note pending-note valid-mani root)))
::
::  Tampered prompt crashes
=/  bad-prompt=@t  `@t`(cat 3 valid-prompt ' INJECTED')
=/  bad-mani
  [query=query results=results prompt=bad-prompt output='evil']
?>  (assert-crash |.((settle-note pending-note bad-mani root)))
::
%pass
