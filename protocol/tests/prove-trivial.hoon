::  tests/prove-trivial.hoon: STARK proof generation smoke test
::
::  Proves the trivial Nock computation [0 1] applied to subject 42.
::  This is Nock's "identity" operation: axis 1 of subject = subject.
::  Result: 42.
::
::  If this compiles, the STARK prover infrastructure works end-to-end:
::  fink:fock traces the execution, generate-proof builds the STARK,
::  and the proof result is not an error.
::
/+  *vesl-prover
::
::  Run the prover on [42 [0 1]] — simplest possible Nock computation
::
=/  result=prove-result:stark-prover
  (prove-computation 42 [0 1])
::
::  Assert success (not an error)
::
?>  ?=(%& -.result)
::
%pass
