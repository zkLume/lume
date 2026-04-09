::  tests/prove-standard.hoon: test standard miner prover directly
::
::  Calls prove:stark-prover with dummy PoW input (version 2).
::  If this works in hoonc but crashes in NockApp, the issue is
::  in the NockApp execution context, not the prover code.
::
/=  *  /common/zeke
/=  stark-prover  /common/stark/prover
/#  softed-constraints
::
::  Use bunt values for the tip5 digests — zero-filled 5-element tuples
::
=/  input=prover-input:stark-prover
  [%2 *noun-digest:tip5 *noun-digest:tip5 10]
=/  result=prove-result:stark-prover
  (prove:stark-prover input)
::
?>  ?=(%& -.result)
::
%pass
