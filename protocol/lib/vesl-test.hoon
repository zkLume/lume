::  lib/vesl-test.hoon: Compile-time testing patterns for Hoon
::
::  Provides reusable assertion arms for NockApp/Nockchain testing.
::  All assertions operate at compile time via hoonc --arbitrary.
::  Compilation success = all tests passed. No runtime needed.
::
::  Usage:
::    /+  *vesl-test
::    ?>  (assert-eq 42 42)
::    ?>  (assert-neq 1 2)
::    ?>  (assert-crash |.((add 'a' ~)))
::    ?>  (assert-hash-eq 0xdead.beef 0xdead.beef)
::    %pass
::
|%
::  +assert-eq: typed equality assertion
::
::  Asserts two values are equal. On failure, crashes with a
::  trace showing "assert-eq failed" via ~| (sigbar).
::  Returns %.y on success for use with ?> chains.
::
::    ?>  (assert-eq (add 2 2) 4)
::
++  assert-eq
  |*  [a=* b=*]
  ~|  'assert-eq: values not equal'
  ?>  =(a b)
  %.y
::
::  +assert-neq: typed inequality assertion
::
::  Asserts two values are NOT equal. On failure, crashes with
::  trace. Returns %.y on success.
::
::    ?>  (assert-neq (hash-leaf 'a') (hash-leaf 'b'))
::
++  assert-neq
  |*  [a=* b=*]
  ~|  'assert-neq: values are equal'
  ?<  =(a b)
  %.y
::
::  +assert-crash: assert a computation crashes
::
::  Wraps the given trap in mule. Asserts the computation
::  produces a %| (failure), meaning it crashed as expected.
::  Returns %.y on success (the crash happened).
::
::  The trap is a |. (bardot) thunk:
::    ?>  (assert-crash |.((settle-note bad-note bad-mani root)))
::
++  assert-crash
  |=  f=(trap *)
  =/  result  (mule f)
  ~|  'assert-crash: computation succeeded (expected crash)'
  ?<  -.result
  %.y
::
::  +assert-ok: assert a computation succeeds (does not crash)
::
::  Wraps the given trap in mule. Asserts the computation
::  produces a %& (success). Returns %.y on success.
::
::    ?>  (assert-ok |.((verify-chunk chunk proof root)))
::
++  assert-ok
  |=  f=(trap *)
  =/  result  (mule f)
  ~|  'assert-ok: computation crashed (expected success)'
  ?>  -.result
  %.y
::
::  +assert-hash-eq: assert two hash atoms are equal
::
::  Same as assert-eq but with a hash-specific trace message.
::  On failure, both hash values appear in the crash trace
::  for debugging cross-VM alignment issues.
::
::    ?>  (assert-hash-eq (hash-leaf 'data') expected-hash)
::
++  assert-hash-eq
  |=  [a=@ b=@]
  ~|  ['assert-hash-eq: hash mismatch' a b]
  ?>  =(a b)
  %.y
::
::  +assert-hash-neq: assert two hash atoms differ
::
::  Verifies two hashes are NOT equal. Used for testing
::  that different inputs produce different digests (collision
::  resistance).
::
::    ?>  (assert-hash-neq (hash-leaf 'a') (hash-leaf 'b'))
::
++  assert-hash-neq
  |=  [a=@ b=@]
  ~|  ['assert-hash-neq: hashes are equal' a]
  ?<  =(a b)
  %.y
::
::  +assert-flag: assert a flag (loobean) is %.y
::
::  Convenience for asserting boolean gates like verify-chunk.
::
::    ?>  (assert-flag (verify-chunk dat proof root))
::
++  assert-flag
  |=  f=?
  ~|  'assert-flag: flag is %.n'
  ?>  f
  %.y
::
::  +assert-not-flag: assert a flag is %.n
::
::  Convenience for asserting boolean gates return false.
::
::    ?>  (assert-not-flag (verify-chunk tampered proof root))
::
++  assert-not-flag
  |=  f=?
  ~|  'assert-not-flag: flag is %.y'
  ?<  f
  %.y
--
