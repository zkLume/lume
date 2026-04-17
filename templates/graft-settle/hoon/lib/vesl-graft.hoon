::  lib/vesl-graft.hoon: gate-agnostic composable verification for any NockApp
::
::  The Graft is a library, not a kernel. It provides:
::    1. A state fragment (vesl-state) you graft onto your kernel state
::    2. A poke dispatcher for %vesl-register/%vesl-settle/%vesl-verify
::    3. A peek helper for querying registered/settled status
::
::  The caller passes a verify-gate — a function that takes opaque data
::  and an expected root, returns a loobean. RAG is one implementation.
::  Any computation type works. No domain-specific imports.
::
::  Usage:
::    /+  *vesl-graft
::    /+  *rag-logic    :: for RAG gate (or your own domain logic)
::    ...your kernel...
::    +$  my-state  [vesl=vesl-state ...your-fields...]
::    ...in poke arm...
::    =/  my-gate=verify-gate
::      |=  [data=* expected-root=@]
::      (verify-manifest ;;(manifest data) expected-root)
::    ?+  -.cause  [~ state]
::      %vesl-register  (vesl-poke vesl.state cause my-gate)
::      %vesl-settle    (vesl-poke vesl.state cause my-gate)
::      %vesl-verify    (vesl-poke vesl.state cause my-gate)
::    ==
::
|%
::  +$vesl-state: the state fragment — graft this onto your kernel
::
::  AUDIT 2026-04-17 H-01: settled set rotates per epoch.
::    epoch          — current epoch number (starts 0, bumped on rotate)
::    registered     — hull-id -> merkle-root (persists across epochs)
::    settled        — current-epoch settled note-ids (replay protection)
::    settle-count   — notes settled in the current epoch
::    prior-settled  — previous epoch's settled set (kept for replay lookback)
::  Replay check walks both `settled` and `prior-settled`, giving a
::  ~2x `epoch-cap` lookback window. When `settle-count` hits
::  `epoch-cap` (below), the next settle rotates: prior-settled :=
::  settled, settled := {new-id}, settle-count := 1, epoch += 1.
::
+$  vesl-state
  $:  epoch=@
      registered=(map @ @)
      settled=(set @)
      settle-count=@
      prior-settled=(set @)
  ==
::
::  +new-state: fresh empty graft state. Use this in your kernel's load arm
::  and anywhere tests need an empty state. Adding fields here (H-01 added
::  epoch/settle-count/prior-settled) doesn't break callers that use this.
::
++  new-state
  ^-  vesl-state
  :*  epoch=0
      registered=*(map @ @)
      settled=*(set @)
      settle-count=0
      prior-settled=*(set @)
  ==
::
::  +epoch-cap: rotation threshold. Matches the pre-H-01 1M cap so
::  per-epoch throughput is unchanged; the cap now triggers rotation
::  instead of permanently bricking. Tunable via edit-and-recompile —
::  did not expose as a cause to keep the poke surface minimal.
::
++  epoch-cap  ^~((mul 1.000 1.000))
::
::  +registered-cap: upper bound on the `registered` map.
::
::  AUDIT 2026-04-17 H-02: without a cap, any caller who can poke
::  %vesl-register cheaply can grow state without bound. 10M is the
::  static cap here — large enough that no legitimate deployment
::  hits it, small enough that a spammer can't brick kernel memory.
::  Future work: signed-envelope registration that requires a
::  capability token, so the cap can be lifted for high-throughput
::  graft operators. Tracked in .dev/FUTURE_WORK.md.
::
++  registered-cap  ^~((mul 10.000 1.000))
::
::  +$graft-payload: generic settlement payload
::
::  note: the settlement note header (id, hull, root, pending state)
::  data: opaque — the verification gate knows the shape.
::       for RAG, this is a manifest. for other domains, anything.
::  expected-root: the Merkle root this data is bound to
::
+$  graft-payload
  $:  note=[id=@ hull=@ root=@ state=[%pending ~]]
      data=*
      expected-root=@
  ==
::
::  +$verify-gate: domain verification gate signature
::
::  Takes note-id + opaque data + expected root, returns loobean.
::  The gate casts data to its domain type (e.g., ;;(manifest data))
::  and performs domain-specific verification.
::
::  AUDIT 2026-04-17 H-03: note-id is now passed to the gate so
::  domain verifiers can enforce `note-id == deterministic-fn(data)`,
::  closing the pre-commit race where an attacker could predict a
::  victim's note-id and settle a different manifest under it first.
::  The graft layer does NOT enforce the binding itself — each
::  domain gate decides what "note-id bound to data" means for it.
::  Gates that don't care can simply ignore the note-id argument.
::
+$  verify-gate  $-([note-id=@ data=* expected-root=@] ?)
::
::  +$vesl-effect: effects the Graft can produce
::
+$  vesl-effect
  $%  [%vesl-registered hull=@ root=@]
      [%vesl-settled note=[id=@ hull=@ root=@ state=[%settled ~]]]
      [%vesl-verified ok=?]
      [%vesl-epoch-rotated old-epoch=@ new-epoch=@]
      [%vesl-error msg=@t]
  ==
::
::  +$vesl-cause: tagged pokes the Graft handles
::
+$  vesl-cause
  $%  [%vesl-register hull=@ root=@]
      [%vesl-settle payload=@]
      [%vesl-verify payload=@]
      [%vesl-rotate-epoch ~]
  ==
::
::  +vesl-poke: dispatch a vesl cause against vesl state
::
::  Takes a verify-gate as third argument — the caller's domain
::  verification function.  Returns [effects updated-state].
::
++  vesl-poke
  |=  [state=vesl-state cause=vesl-cause veri=verify-gate]
  ^-  [(list vesl-effect) vesl-state]
  ?-  -.cause
    ::
    ::  %vesl-register — store hull root
    ::
      %vesl-register
    ::  Guard: reject re-registration (hull already has a root; M-01)
    ::
    ?:  (~(has by registered.state) hull.cause)
      :_  state
      ~[[%vesl-error 'vesl-graft: hull already registered']]
    ::  Guard: registered map capacity (H-02)
    ::
    ?:  (gte ~(wyt by registered.state) registered-cap)
      :_  state
      ~[[%vesl-error 'vesl-graft: registered map at capacity']]
    =/  new-reg  (~(put by registered.state) hull.cause root.cause)
    :_  state(registered new-reg)
    ~[[%vesl-registered hull.cause root.cause]]
    ::
    ::  %vesl-settle — cue payload, validate, verify via gate, settle
    ::    Guards: root registered, roots match, replay protection.
    ::    AUDIT 2026-04-17 H-01: no permanent cap. At `epoch-cap`, the
    ::    settled set rotates — prior-settled := settled, settled := {id},
    ::    epoch += 1. Replay check covers both sets, so the lookback
    ::    window is ~2x epoch-cap.
    ::    Crash semantics: ?> on gate failure = unprovable STARK.
    ::
      %vesl-settle
    =/  raw=*  (cue payload.cause)
    =/  args=graft-payload  ;;(graft-payload raw)
    ::  Guard: reject unregistered roots
    ::
    ?.  (~(has by registered.state) hull.note.args)
      :_  state
      ~[[%vesl-error 'vesl-graft: root not registered']]
    ::  Guard: expected root must match registered root
    ::
    ?.  =(expected-root.args (~(got by registered.state) hull.note.args))
      :_  state
      ~[[%vesl-error 'vesl-graft: root mismatch']]
    ::  Guard: note header root must match expected root
    ::
    ?.  =(root.note.args expected-root.args)
      :_  state
      ~[[%vesl-error 'vesl-graft: note root does not match expected root']]
    ::  Guard: replay protection — covers current AND prior epoch
    ::
    ?:  (~(has in settled.state) id.note.args)
      :_  state
      ~[[%vesl-error 'vesl-graft: note already settled']]
    ?:  (~(has in prior-settled.state) id.note.args)
      :_  state
      ~[[%vesl-error 'vesl-graft: note already settled (prior epoch)']]
    ::  Verify via caller's gate — crash on failure
    ::
    ?>  (veri id.note.args data.args expected-root.args)
    ::  Apply settlement. Rotate iff the current epoch is already at cap.
    ::
    =/  at-cap=?  (gte settle-count.state epoch-cap)
    ?.  at-cap
      =/  new-settled  (~(put in settled.state) id.note.args)
      :_  state(settled new-settled, settle-count +(settle-count.state))
      ~[[%vesl-settled note=[id.note.args hull.note.args root.note.args [%settled ~]]]]
    ::  Rotation: prior-settled := settled; settled := {new-id}
    ::
    =/  old-epoch  epoch.state
    =/  new-epoch  +(old-epoch)
    =/  rotated
      %=  state
        epoch          new-epoch
        prior-settled  settled.state
        settled        (~(put in *(set @)) id.note.args)
        settle-count   1
      ==
    :_  rotated
    :~  [%vesl-epoch-rotated old-epoch new-epoch]
        [%vesl-settled note=[id.note.args hull.note.args root.note.args [%settled ~]]]
    ==
    ::
    ::  %vesl-verify — pure verification, no state transition
    ::    Returns [%vesl-verified %.y] or [%vesl-verified %.n].
    ::
      %vesl-verify
    =/  raw=*  (cue payload.cause)
    =/  args=graft-payload  ;;(graft-payload raw)
    ::  Check registration
    ::
    ?.  (~(has by registered.state) hull.note.args)
      :_  state
      ~[[%vesl-verified %.n]]
    ::  Guard: expected root must match registered root
    ::
    ?.  =(expected-root.args (~(got by registered.state) hull.note.args))
      :_  state
      ~[[%vesl-verified %.n]]
    ::  Guard: note header root must match expected root (H-04 parity
    ::  with %vesl-settle). Without this, a caller polling verify can
    ::  get a green light and then watch settle crash on a field verify
    ::  never inspected.
    ::
    ?.  =(root.note.args expected-root.args)
      :_  state
      ~[[%vesl-verified %.n]]
    ::  Verify via caller's gate — soft failure (no crash)
    ::
    =/  ok=?  (veri id.note.args data.args expected-root.args)
    :_  state
    ~[[%vesl-verified ok]]
    ::
    ::  %vesl-rotate-epoch — force rotation now (admin-initiated)
    ::    No-auth by design: anyone who can poke the graft can force
    ::    rotation. Worst-case impact: truncates the lookback window
    ::    earlier than the count-based trigger would. Replay protection
    ::    within the new current + prior pair remains intact.
    ::
      %vesl-rotate-epoch
    =/  old-epoch  epoch.state
    =/  new-epoch  +(old-epoch)
    =/  rotated
      %=  state
        epoch          new-epoch
        prior-settled  settled.state
        settled        *(set @)
        settle-count   0
      ==
    :_  rotated
    ~[[%vesl-epoch-rotated old-epoch new-epoch]]
  ==
::
::  +vesl-peek: query vesl state by path
::
::  Returns ~ for unrecognized paths (pass through to your kernel's peek).
::  Returns ``(unit) for recognized paths.
::
++  vesl-peek
  |=  [state=vesl-state =path]
  ^-  (unit (unit *))
  ?+  path  ~
    [%registered hull=@ ~]
      =/  vid  +<.path
      ``(~(has by registered.state) vid)
    ::
    [%settled note-id=@ ~]
      =/  nid  +<.path
      ::  Replay lookup must cover current + prior epoch.
      ::
      ?:  (~(has in settled.state) nid)  ``%.y
      ``(~(has in prior-settled.state) nid)
    ::
    [%root hull=@ ~]
      =/  vid  +<.path
      ``(~(get by registered.state) vid)
    ::
    [%epoch ~]  ``epoch.state
    ::
    [%settle-count ~]  ``settle-count.state
  ==
--
