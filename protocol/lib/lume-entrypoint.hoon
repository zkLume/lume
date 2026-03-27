::  protocol/lib/lume-entrypoint.hoon: ABI Boundary for the Lume ZK-Prover
::
::  Universal entrypoint gate. Accepts a single jammed atom (@)
::  from the off-chain Rust Vessel, deserializes via cue, validates
::  via strict mold (;;), and runs settlement logic.
::
::  ABI contract: Rust sends  jam([note manifest root])
::  This gate: cue -> mold -> settle -> settled note or crash
::
::  Defense-in-depth: malformed payloads crash at the mold boundary
::  BEFORE reaching any logic gates. The Nock VM enforces this.
::
/-  *lume
/+  *lume-logic
::
|%
::
::  +lume-entrypoint: universal ABI wrapper
::
::  Single atom in, settled note out (or crash).
::  Three-phase pipeline:
::    1. cue: deserialize jammed atom to raw noun
::    2. ;;:  strict mold — validate noun structure against
::           settlement-payload type. Crashes on any mismatch.
::    3. settle-note: verify manifest + transition state
::
++  lume-entrypoint
  |=  payload=@
  ^-  [id=@ vessel=@ root=@ state=[%settled ~]]
  =/  raw=*  (cue payload)
  =/  args=settlement-payload  ;;(settlement-payload raw)
  (settle-note note.args mani.args expected-root.args)
--
