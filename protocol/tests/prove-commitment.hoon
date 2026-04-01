::  tests/prove-commitment.hoon: STARK proof of note commitment tuple
::
::  Proves *[[note-id hull-id root] [0 1]] — the same computation
::  that the kernel's %prove handler would execute.
::
::  Subject is a cell of [id=1 hull=7 root=12345], simulating
::  a real settlement commitment with a small root placeholder.
::
/+  *vesl-prover
::
=/  commitment  [1 7 12.345]
=/  result=prove-result:stark-prover
  (prove-computation commitment [0 1])
::
?>  ?=(%& -.result)
::
%pass
