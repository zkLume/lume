::  protocol/lib/vesl-kernel.hoon: NockApp kernel for settlement + proving
::
::  Proper NockApp kernel with versioned state, poke, peek, and load arms.
::  Poke causes:
::    [%register hull=@ root=@]  — register a hull root
::    [%settle payload=@]          — verify manifest + settle note
::    [%prove  payload=@]          — settle + generate STARK proof
::    [%sig-hash seeds-jam=@ fee=@] — compute sig-hash from seeds + fee
::    [%tx-id spends-jam=@]        — compute tx-id from spends
::
::  The settle/prove payload is a jammed settlement-payload atom.
::  The sig-hash/tx-id pokes use Hoon's tx-engine hashable infrastructure
::  to produce byte-exact hashes for Rust-assembled transactions.
::
::  Security hardening:
::    - %settle/%prove reject unregistered roots (must %register first)
::    - %settle/%prove reject duplicate note IDs (replay protection)
::
::  Compiled: hoonc --new protocol/lib/vesl-kernel.hoon hoon/
::
/-  *vesl
/+  *rag-logic
/+  *vesl-prover
/+  *vesl-merkle
/=  *  /common/wrapper
/=  txv1  /common/tx-engine-1
::
=>
|%
::  Kernel state — tracks registered roots and settled notes
::
+$  versioned-state
  $:  %v1
      registered=(map @ @)
      settled=(set @)
  ==
::  Effects the kernel can produce
::
+$  effect  *
::  Causes the kernel accepts
::
+$  cause
  $%  [%register hull=@ root=@]
      [%settle payload=@]
      [%prove payload=@]
      [%sig-hash seeds-jam=@ fee=@]
      [%tx-id spends-jam=@]
      [%diag-cue seeds-jam=@]
      [%diag-sieve seeds-jam=@]
      [%diag-hash seeds-jam=@ fee=@]
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
  ::
  ++  peek
    |=  =path
    ^-  (unit (unit *))
    ?+  path  ~
      [%registered hull=@ ~]
        =/  vid  +<.path
        ``(~(has by registered.state) vid)
      ::
      [%settled note-id=@ ~]
        =/  nid  +<.path
        ``(~(has in settled.state) nid)
    ==
  ::
  ++  poke
    |=  =ovum:moat
    ^-  [(list effect) _state]
    =/  act  ((soft cause) cause.input.ovum)
    ?~  act
      ~>  %slog.[3 'vesl: invalid cause']
      [~ state]
    ?-  -.u.act
      ::
      ::  %register — store hull root, return confirmation
      ::
        %register
      ::  Guard: reject re-registration (hull already has a root)
      ::
      ?:  (~(has by registered.state) hull.u.act)
        ~>  %slog.[3 'vesl: hull already registered']
        [~ state]
      =/  new-reg  (~(put by registered.state) hull.u.act root.u.act)
      :_  state(registered new-reg)
      ^-  (list effect)
      ~[[%registered hull.u.act root.u.act]]
      ::
      ::  %settle — verify manifest and transition note to %settled
      ::    Guards: root must be registered, note ID must not be settled
      ::
        %settle
      =/  raw=*  (cue payload.u.act)
      =/  args=settlement-payload  ;;(settlement-payload raw)
      ::  Guard: reject unregistered roots
      ::
      ?.  (~(has by registered.state) hull.note.args)
        ~>  %slog.[3 'vesl: root not registered']
        [~ state]
      ::  Guard: expected root must match registered root
      ::
      ?.  =(expected-root.args (~(got by registered.state) hull.note.args))
        ~>  %slog.[3 'vesl: root mismatch']
        [~ state]
      ::  Guard: note header root must match expected root (H-07)
      ::
      ?.  =(root.note.args expected-root.args)
        ~>  %slog.[3 'vesl: note root does not match expected root']
        [~ state]
      ::  Guard: reject duplicate note IDs (replay protection)
      ::
      ?:  (~(has in settled.state) id.note.args)
        ~>  %slog.[3 'vesl: note already settled (replay rejected)']
        [~ state]
      =/  result  (settle-note note.args mani.args expected-root.args)
      =/  new-settled  (~(put in settled.state) id.note.args)
      :_  state(settled new-settled)
      ^-  (list effect)
      ~[result]
      ::
      ::  %prove — settle + generate STARK proof (atomic)
      ::    Guards: same as %settle
      ::    If proving crashes, nothing settles. Use %settle for
      ::    settlement without proof.
      ::
        %prove
      =/  raw=*  (cue payload.u.act)
      =/  args=settlement-payload  ;;(settlement-payload raw)
      ::  Guard: reject unregistered roots
      ::
      ?.  (~(has by registered.state) hull.note.args)
        ~>  %slog.[3 'vesl: root not registered']
        [~ state]
      ::  Guard: expected root must match registered root
      ::
      ?.  =(expected-root.args (~(got by registered.state) hull.note.args))
        ~>  %slog.[3 'vesl: root mismatch']
        [~ state]
      ::  Guard: note header root must match expected root (H-07)
      ::
      ?.  =(root.note.args expected-root.args)
        ~>  %slog.[3 'vesl: note root does not match expected root']
        [~ state]
      ::  Guard: reject duplicate note IDs (replay protection)
      ::
      ?:  (~(has in settled.state) id.note.args)
        ~>  %slog.[3 'vesl: note already settled (replay rejected)']
        [~ state]
      ::  Verify manifest (must pass before we attempt proving)
      ::
      =/  result-note  (settle-note note.args mani.args expected-root.args)
      ::  Phase 3: field-safe STARK execution on manifest data
      ::
      ::  Decompose all text fields to 7-byte belt lists, then
      ::  fold to a single atom < Goldilocks prime (sum mod p).
      ::  Cell subjects crash the STARK memory table — the table
      ::  decomposes the full subject tree and can't represent
      ::  cell nodes as field elements.
      ::
      ::  Root/hull bound via Fiat-Shamir header/nonce (Phase 1).
      ::  Belt digest bound via STARK execution trace.
      ::
      ::  Formula: 64 nested increments (Nock 0/4 only).
      ::  Subject: belt-digest (single atom < p).
      ::  Product: belt-digest + 64.
      ::
      =/  qb=(list @)  (split-to-belts query.mani.args)
      =/  ob=(list @)  (split-to-belts output.mani.args)
      =/  pb=(list @)  (split-to-belts prompt.mani.args)
      =/  chunk-belts=(list @)
        =|  acc=(list @)
        =/  res  results.mani.args
        |-
        ?~  res  (flop acc)
        $(acc (weld (flop (split-to-belts dat.chunk.i.res)) acc), res t.res)
      =/  all-belts=(list @)
        (weld qb (weld ob (weld pb chunk-belts)))
      ::  fold all belts to single atom < p
      ::  p = 2^64 - 2^32 + 1 (Goldilocks prime)
      ::
      =/  p=@  (add (sub (bex 64) (bex 32)) 1)
      =/  belt-digest=@
        %+  roll  all-belts
        |=  [a=@ b=@]
        (mod (add a b) p)
      ::  64 nested increments on [0 1]
      ::  known-working pattern: atom subject + Nock 0/4 only
      ::
      ::  TODO: AUDIT 2026-04-17 C-lead-1 — STARK formula hardcoding
      ::
      ::  The proved formula below is a hardcoded 64-nested-increment
      ::  `[4 [4 [4 ... [4 [0 1]]]]]`. No version tag, no length prefix,
      ::  no explicit commitment to the formula shape is absorbed into
      ::  the Fiat-Shamir transcript beyond the root/hull digests set by
      ::  vesl-prover. If the Rust prover's opcode count or structure
      ::  silently drifts (refactor, jet change, upstream nockvm update),
      ::  the verifier could accept a STARK that proves a *different*
      ::  formula without raising an error.
      ::
      ::  Suggested fix (requires STARK-fluent reviewer):
      ::    Hash the formula noun and absorb the digest into the
      ::    Fiat-Shamir transcript before challenge derivation, or
      ::    prepend a version tag to the proof header. Do not edit
      ::    this site without cross-VM prove->verify on a perturbed
      ::    formula to confirm binding holds.
      ::
      ::  See .dev/CRITICAL_LEADS.md.
      ::
      =/  fs-formula=*
        =/  f=*  [0 1]
        =|  i=@
        |-
        ?:  =(i 64)  f
        $(f [4 f], i +(i))
      =/  proof-attempt
        %-  mule  |.
        (prove-computation belt-digest fs-formula expected-root.args hull.note.args)
      ?.  -.proof-attempt
        ::  Proof FAILED — jam the trace for Rust-side decoding
        ::
        ~>  %slog.[3 'vesl: prove-computation crashed']
        :_  state
        ^-  (list effect)
        ~[[%prove-failed (jam p.proof-attempt)]]
      ::  Proof succeeded -- settle and return [result-note proof]
      ::
      =/  new-settled  (~(put in settled.state) id.note.args)
      :_  state(settled new-settled)
      ^-  (list effect)
      ~[[result-note p.proof-attempt]]
      ::
      ::  %sig-hash — compute sig-hash from jammed seeds + fee
      ::    Uses tx-engine's hashable infrastructure for byte-exact hashes.
      ::    Stateless: does not modify kernel state.
      ::
        %sig-hash
      ::  AUDIT 2026-04-19 H-08: wrap cue+sieve+hash chain in mule so a
      ::  malformed seeds-jam or shape-incompatible noun yields a typed
      ::  error effect instead of crashing the kernel.
      ::
      =/  attempt
        %-  mule  |.
        =/  sds=seeds:txv1  ;;(seeds:txv1 (cue seeds-jam.u.act))
        ^-  hash:txv1
        %-  hash-hashable:tip5
        [(sig-hashable:seeds:txv1 sds) leaf+fee.u.act]
      :_  state
      ^-  (list effect)
      ?:  -.attempt
        ~[[%sig-hash p.attempt]]
      ~[[%sig-hash-error (jam p.attempt)]]
      ::
      ::  %tx-id — compute tx-id from jammed spends
      ::    Uses tx-engine's hashable infrastructure for byte-exact hashes.
      ::    Stateless: does not modify kernel state.
      ::
        %tx-id
      ::  AUDIT 2026-04-19 H-08: mirror %sig-hash — mule-wrap cue+sieve+hash
      ::  so malformed spends-jam emits a typed error instead of crashing.
      ::
      =/  attempt
        %-  mule  |.
        =/  sps=spends:txv1  ;;(spends:txv1 (cue spends-jam.u.act))
        ^-  tx-id:txv1
        %-  hash-hashable:tip5
        [leaf+%1 (hashable:spends:txv1 sps)]
      :_  state
      ^-  (list effect)
      ?:  -.attempt
        ~[[%tx-id p.attempt]]
      ~[[%tx-id-error (jam p.attempt)]]
      ::
      ::  %diag-cue — CUE seeds JAM without sieve, report noun shape.
      ::    Diagnostic: isolates CUE from type validation.
      ::
        %diag-cue
      ::  AUDIT 2026-04-19 H-08: wrap cue in mule — a malformed jam bunt
      ::  otherwise crashes the kernel from this diagnostic arm.
      ::
      =/  attempt  (mule |.((cue seeds-jam.u.act)))
      :_  state
      ^-  (list effect)
      ?:  -.attempt
        =/  raw=*  p.attempt
        =/  is-cell=?  ?=(^ raw)
        ~[[%diag-cue is-cell raw]]
      ~[[%diag-cue-error (jam p.attempt)]]
      ::
      ::  %diag-sieve — CUE + sieve inside mule, catch crash.
      ::    Diagnostic: determines if ;;(seeds:txv1 ...) is the crash site.
      ::
        %diag-sieve
      ::  AUDIT 2026-04-19 H-08: bring cue inside mule — the earlier wrap
      ::  protected the sieve but not the cue, so a malformed jam still
      ::  crashed before reaching the sieve.
      ::
      =/  attempt
        %-  mule  |.
        ;;(seeds:txv1 (cue seeds-jam.u.act))
      :_  state
      ^-  (list effect)
      ?:  -.attempt
        ~[[%diag-sieve %fail (jam p.attempt)]]
      ~[[%diag-sieve %ok ~]]
      ::
      ::  %diag-hash — full sig-hash computation inside mule
      ::
        %diag-hash
      ::  AUDIT 2026-04-19 H-08: pull the cue+sieve into the mule — an
      ::  outer ;;(seeds:txv1 ...) on a malformed cue crashes before the
      ::  hash attempt had a chance to catch.
      ::
      =/  attempt
        %-  mule  |.
        =/  sds=seeds:txv1  ;;(seeds:txv1 (cue seeds-jam.u.act))
        %-  hash-hashable:tip5
        [(sig-hashable:seeds:txv1 sds) leaf+fee.u.act]
      ?:  -.attempt
        :_  state
        ^-  (list effect)
        ~[[%diag-hash %ok p.attempt]]
      :_  state
      ^-  (list effect)
      ~[[%diag-hash %fail (jam p.attempt)]]
    ==
  --
--
((moat |) inner)
