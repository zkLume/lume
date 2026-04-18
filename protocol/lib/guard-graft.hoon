::  lib/guard-graft.hoon: hull-keyed root trellis with leaf-hash check
::
::  The Graft is a library, not a kernel. It provides:
::    1. A state fragment (guard-state) you graft onto your kernel state
::    2. A poke dispatcher for %guard-register / %guard-check
::    3. A peek helper for querying registered roots by hull
::
::  Guard is the middle commitment tier:
::    mint-graft  — commit a root under a hull-id. No verify.
::    guard-graft — commit a root + verify hash-leaf(data) == root.
::                  No gate, no replay protection.
::    settle-graft  — full verify-gate lifecycle with replay protection.
::
::  Registration is one-shot per hull, mirroring mint-graft and
::  settle-graft (AUDIT 2026-04-17 M-01). Once a hull-id is registered
::  with a root, that mapping is immutable for the lifetime of the
::  graft state. A signature-gated revoke/rotate cause is future work.
::
::  Usage:
::    /+  *guard-graft
::    ...your kernel...
::    +$  my-state  [guard=guard-state ...your-fields...]
::    ...in poke arm...
::    ?+  -.cause  [~ state]
::      %guard-register  (guard-poke guard.state cause)
::      %guard-check     (guard-poke guard.state cause)
::    ==
::
/+  *vesl-merkle
|%
::  +$guard-state: the state fragment — graft this onto your kernel
::
::    roots — hull-id -> merkle-root (append-only)
::
+$  guard-state
  $:  roots=(map @ @)
  ==
::
::  +new-state: fresh empty graft state
::
++  new-state
  ^-  guard-state
  :*  roots=*(map @ @)
  ==
::
::  +roots-cap: upper bound on the `roots` map.
::
::  Mirror of settle-graft's registered-cap (AUDIT 2026-04-17 H-02).
::  Without a cap, a caller who can poke %guard-register cheaply can
::  grow state without bound. 10M is the static cap.
::
++  roots-cap  ^~((mul 10.000 1.000))
::
::  +$guard-effect: effects the Graft can produce
::
+$  guard-effect
  $%  [%guard-registered hull=@ root=@]
      [%guard-checked hull=@ ok=?]
      [%guard-error msg=@t]
  ==
::
::  +$guard-cause: tagged pokes the Graft handles
::
+$  guard-cause
  $%  [%guard-register hull=@ root=@]
      [%guard-check hull=@ data=@]
  ==
::
::  +guard-poke: dispatch a guard cause against guard state
::
++  guard-poke
  |=  [state=guard-state cause=guard-cause]
  ^-  [(list guard-effect) guard-state]
  ?-  -.cause
    ::
    ::  %guard-register — store hull root (append-only)
    ::
      %guard-register
    ::  Guard: reject re-registration (hull already has a root)
    ::
    ?:  (~(has by roots.state) hull.cause)
      :_  state
      ~[[%guard-error 'guard-graft: hull already registered']]
    ::  Guard: roots map capacity
    ::
    ?:  (gte ~(wyt by roots.state) roots-cap)
      :_  state
      ~[[%guard-error 'guard-graft: roots map at capacity']]
    =/  new-roots  (~(put by roots.state) hull.cause root.cause)
    :_  state(roots new-roots)
    ~[[%guard-registered hull.cause root.cause]]
    ::
    ::  %guard-check — verify hash-leaf(data) == registered root
    ::
    ::  Soft check: unregistered hull returns %guard-error (not a
    ::  crash). Successful hash match emits [%guard-checked hull %.y];
    ::  mismatch emits [%guard-checked hull %.n]. Callers that want
    ::  crash-on-bad-leaf semantics should use settle-graft with its
    ::  verify-gate instead.
    ::
      %guard-check
    ?.  (~(has by roots.state) hull.cause)
      :_  state
      ~[[%guard-error 'guard-graft: hull not registered']]
    =/  expected=@  (~(got by roots.state) hull.cause)
    =/  got=@  (hash-leaf data.cause)
    =/  ok=?  =(got expected)
    :_  state
    ~[[%guard-checked hull.cause ok]]
  ==
::
::  +guard-peek: query guard state by path
::
::  Returns ~ for unrecognized paths (pass through to your kernel's peek).
::
++  guard-peek
  |=  [state=guard-state =path]
  ^-  (unit (unit *))
  ?+  path  ~
    [%guard-root hull=@ ~]
      =/  vid  +<.path
      ``(~(get by roots.state) vid)
  ==
--
