::  lib/vesl-prover.hoon: STARK proof generation for arbitrary Nock computation
::
::  Forked from nock-prover.hoon to bypass puzzle-nock.  The standard
::  prover is PoW-specific: it derives [subject formula] from
::  puzzle-nock(header, nonce, pow-len).  Vesl needs to prove
::  arbitrary Nock computations (e.g. settle-note execution).
::
::  This prover:
::  1. Accepts [subject formula] directly
::  2. Traces execution via fink:fock
::  3. Generates a STARK proof via the same constraint system
::  4. Embeds a zero puzzle commitment (verifier skips puzzle check)
::
/=  compute-table-v2  /common/v2/table/prover/compute
/=  memory-table-v2  /common/v2/table/prover/memory
/=  nock-common-v2  /common/v2/nock-common
/=  *  /common/zeke
/=  stark-prover  /common/stark/prover
/#  softed-constraints
::
|%
::
::  +prover-core: STARK prover initialised with softed constraints
::
++  prover-core
  =|  in=stark-input
  =/  sc=stark-config
    %*  .  *stark-config
      prep  softed-constraints
    ==
  %_    stark-prover
      +<+<
    %_  in
      stark-config  sc
    ==
  ==
::
::  +prove-computation: Generate STARK proof of any Nock [subject formula]
::
::  Bypasses puzzle-nock entirely.  The STARK constraints only check
::  correct Nock VM execution, not which program was run.
::
++  prove-computation
  |=  [subject=* formula=*]
  ^-  prove-result:stark-prover
  ::  1. trace the nock execution
  ::
  =/  [prod=* return=fock-return]
    (fink:fock [subject formula])
  ::  2. extract v2 preprocessed constraints
  ::
  =/  pre=preprocess-data
    p.pre-2.softed-constraints
  ::  3. call generate-proof via prove-door from initialised core
  ::
  ::  Zero puzzle commitment — no PoW.  Vesl verifier identifies
  ::  the proof by the embedded [subject formula prod], not by
  ::  a puzzle derivation.
  ::
  %-  %~  generate-proof
        prove-door:prover-core
      :*  nock-common-v2
          funcs:compute-table-v2
          static:common:compute-table-v2
          funcs:memory-table-v2
          static:common:memory-table-v2
          pre
      ==
  :*  %2
      *noun-digest:tip5
      *noun-digest:tip5
      0
      subject
      formula
      prod
      return
  ==
::
::  +prove-standard: pass-through to standard PoW prover
::
++  prove-standard
  |=  input=prover-input:stark-prover
  (prove:prover-core input)
--
