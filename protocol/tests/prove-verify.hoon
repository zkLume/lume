::  tests/prove-verify.hoon: STARK proof generation + full math verification
::
::  End-to-end: generate a STARK proof with vesl-prover, then verify
::  it with full STARK math (FRI, linking-checks, constraint satisfaction,
::  DEEP polynomial checks) via vesl-verifier.
::
/+  *vesl-prover
/+  vesl-verifier
::
::  1. Generate STARK proof of [42 [0 1]] — identity computation
::
=/  result=prove-result:stark-prover
  (prove-computation 42 [0 1])
?>  ?=(%& -.result)
::
::  2. Structural checks (Level 1)
::
::  version must be %2
?>  ?=([%& %2 *] result)
::
::  extract proof via axis (proven workaround for each face issue)
=/  prf=proof  p.result
::
::  structural re-execution check
?>  (verify-structure:vesl-verifier prf 42 [0 1])
::
::  3. Full STARK math verification (Level 2)
::
::  verify:vesl-verifier calls the forked verifier which accepts
::  [s f] directly instead of deriving from puzzle-nock.
::  This exercises the full FRI + linking-checks + constraint
::  polynomial + DEEP codeword verification pipeline.
::
?>  (verify:vesl-verifier prf ~ 0 42 [0 1])
::
%pass
