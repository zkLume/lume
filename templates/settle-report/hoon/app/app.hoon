::  settle-report — computation settlement NockApp
::
::  The settlement pattern: commit to an expected result, then
::  submit the actual computation for verification. If the hash
::  matches, it settles. If not, rejected. Can't settle twice.
::
::  This is the simplified version of what Vesl does for RAG
::  verification. The full version adds Merkle proofs and STARK
::  proving, but the core pattern is identical: commit, verify,
::  settle, guard against replay.
::
::  Demonstrates:
::    - commitment/settlement lifecycle
::    - three-guard verification:
::        1. commitment must exist
::        2. no duplicate settlements (replay protection)
::        3. hash must match commitment
::    - rejection effects with reason codes
::    - set-based state (settled IDs)
::
::  Compile: hoonc hoon/app/app.hoon $NOCK_HOME/hoon/
::
/+  lib
/=  *  /common/wrapper
::
=>
|%
::  kernel state — commitments and settlements
::
+$  versioned-state
  $:  %v1
      commitments=(map @ @)
      settlements=(set @)
  ==
::
+$  effect  *
::
+$  cause
  $%  [%commit id=@ dat=@]
      [%settle id=@ dat=@]
  ==
--
|%
++  moat  (keep versioned-state)
::
++  inner
  |_  state=versioned-state
  ::
  ++  load
    |=  old-state=versioned-state
    ^-  _state
    old-state
  ::  +peek: query commitment and settlement status
  ::    /committed/<id> -> %.y if commitment exists
  ::    /settled/<id>   -> %.y if already settled
  ::
  ++  peek
    |=  =path
    ^-  (unit (unit *))
    ?+  path  ~
      [%committed id=@ ~]
        =/  cid  +<.path
        ``(~(has by commitments.state) cid)
      [%settled id=@ ~]
        =/  sid  +<.path
        ``(~(has in settlements.state) sid)
    ==
  ::  +poke: commit or settle
  ::
  ++  poke
    |=  =ovum:moat
    ^-  [(list effect) _state]
    =/  act  ((soft cause) cause.input.ovum)
    ?~  act
      ~>  %slog.[3 'settle: invalid cause']
      [~ state]
    ?-  -.u.act
      ::  %commit — register a commitment
      ::    stores hash of data for later verification
      ::
        %commit
      =/  hash=@  (shax dat.u.act)
      =/  new-com  (~(put by commitments.state) id.u.act hash)
      ~>  %slog.[0 'settle: commitment registered']
      :_  state(commitments new-com)
      ~[[%committed id.u.act hash]]
      ::  %settle — verify data and settle
      ::    three guards, in order:
      ::      1. commitment must exist for this id
      ::      2. id must not already be settled
      ::      3. hash of submitted data must match commitment
      ::
        %settle
      ::  guard 1: commitment must exist
      ::
      ?.  (~(has by commitments.state) id.u.act)
        ~>  %slog.[3 'settle: no commitment for id']
        :_  state
        ~[[%rejected id.u.act 'no commitment']]
      ::  guard 2: replay protection
      ::
      ?:  (~(has in settlements.state) id.u.act)
        ~>  %slog.[3 'settle: already settled']
        :_  state
        ~[[%rejected id.u.act 'already settled']]
      ::  guard 3: hash verification
      ::
      =/  expected=@  (~(got by commitments.state) id.u.act)
      =/  actual=@  (shax dat.u.act)
      ?.  =(expected actual)
        ~>  %slog.[3 'settle: hash mismatch']
        :_  state
        ~[[%rejected id.u.act 'hash mismatch']]
      ::  all guards passed — settle
      ::
      =/  new-set  (~(put in settlements.state) id.u.act)
      ~>  %slog.[0 'settle: confirmed']
      :_  state(settlements new-set)
      ~[[%settled id.u.act actual]]
    ==
  --
--
((moat |) inner)
