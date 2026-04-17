::  lib/mint-graft.hoon: hull-keyed commitment trellis
::
::  The Graft is a library, not a kernel. It provides:
::    1. A state fragment (mint-state) you graft onto your kernel state
::    2. A poke dispatcher for %mint-commit
::    3. A peek helper for querying committed roots by hull
::
::  Mint only stores the commitment. No gate, no verify, no settlement.
::  Consumers that want to verify data against a committed root go
::  through the Guard graft (hash-leaf check against the stored root)
::  or Settle (full register + settle lifecycle with a verify-gate).
::
::  Commitment is one-shot per hull. Once a hull-id is committed to a
::  root, that mapping is immutable for the lifetime of the graft
::  state — roots are permanent commitments. Legitimate key rotation
::  currently requires a fresh deployment. A signature-gated
::  `%mint-revoke` / `%mint-rotate` cause is tracked as future work.
::
::  Usage:
::    /+  *mint-graft
::    ...your kernel...
::    +$  my-state  [mint=mint-state ...your-fields...]
::    ...in poke arm...
::    ?+  -.cause  [~ state]
::      %mint-commit  (mint-poke mint.state cause)
::    ==
::
|%
::  +$mint-state: the state fragment — graft this onto your kernel
::
::    commits — hull-id -> merkle-root (append-only)
::
+$  mint-state
  $:  commits=(map @ @)
  ==
::
::  +new-state: fresh empty graft state. Use this in your kernel's load
::  arm and anywhere tests need an empty state.
::
++  new-state
  ^-  mint-state
  :*  commits=*(map @ @)
  ==
::
::  +commits-cap: upper bound on the `commits` map.
::
::  Mirror of vesl-graft's registered-cap (AUDIT 2026-04-17 H-02).
::  Without a cap, any caller who can poke %mint-commit cheaply can
::  grow state without bound. 10M is the static cap — large enough
::  that no legitimate deployment hits it, small enough that a
::  spammer can't brick kernel memory. Future work: signed-envelope
::  commitment that requires a capability token so the cap can be
::  lifted for high-throughput operators.
::
++  commits-cap  ^~((mul 10.000 1.000))
::
::  +$mint-effect: effects the Graft can produce
::
+$  mint-effect
  $%  [%mint-committed hull=@ root=@]
      [%mint-error msg=@t]
  ==
::
::  +$mint-cause: tagged pokes the Graft handles
::
+$  mint-cause
  $%  [%mint-commit hull=@ root=@]
  ==
::
::  +mint-poke: dispatch a mint cause against mint state
::
++  mint-poke
  |=  [state=mint-state cause=mint-cause]
  ^-  [(list mint-effect) mint-state]
  ?-  -.cause
    ::
    ::  %mint-commit — store hull root (append-only)
    ::
      %mint-commit
    ::  Guard: reject re-commit (hull already has a root)
    ::
    ?:  (~(has by commits.state) hull.cause)
      :_  state
      ~[[%mint-error 'mint-graft: hull already committed']]
    ::  Guard: commits map capacity
    ::
    ?:  (gte ~(wyt by commits.state) commits-cap)
      :_  state
      ~[[%mint-error 'mint-graft: commits map at capacity']]
    =/  new-commits  (~(put by commits.state) hull.cause root.cause)
    :_  state(commits new-commits)
    ~[[%mint-committed hull.cause root.cause]]
  ==
::
::  +mint-peek: query mint state by path
::
::  Returns ~ for unrecognized paths (pass through to your kernel's peek).
::  Returns ``(unit) for recognized paths.
::
++  mint-peek
  |=  [state=mint-state =path]
  ^-  (unit (unit *))
  ?+  path  ~
    [%mint-commit hull=@ ~]
      =/  vid  +<.path
      ``(~(get by commits.state) vid)
  ==
--
