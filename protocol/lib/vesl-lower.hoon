::  lib/vesl-lower.hoon: Nock formula lowering pass (9/10/11 → 0-8)
::
::  Eliminates opcodes 9, 10, 11 from any Nock formula,
::  producing an equivalent formula using only opcodes 0-8.
::  Required for STARK proving — fink:fock only supports
::  opcodes 0-8 + autocons.
::
::  Lowering rules:
::    [9 b c]      → [7 c' [2 [0 1] [0 b]]]   arm call → compose + eval
::    [10 [b c] d] → [8 [d' c'] (edit b)]       axis edit → push + reconstruct
::    [11 [b c] d] → d'                          dynamic hint → strip
::    [11 b c]     → c'                          static hint → strip
::
::  where x' = lower(x)
::
::  This lowers the formula tree only.  Formulas embedded as
::  data in the subject (e.g., core batteries via Nock 1) must
::  be lowered separately before STARK execution.
::
|%
::  +lower: rewrite a Nock formula to use only opcodes 0-8
::
++  lower
  |=  f=*
  ^-  *
  ::  atom: pass through
  ::
  ?@  f  f
  =/  op  -.f
  ::  autocons: head is a cell, lower both sides
  ::
  ?^  op
    [(lower -.f) (lower +.f)]
  ::  opcode dispatch
  ::
  ?+  op  f
    %0  f
    %1  f
    %2  [%2 (lower -.+.f) (lower +.+.f)]
    %3  [%3 (lower +.f)]
    %4  [%4 (lower +.f)]
    %5  [%5 (lower -.+.f) (lower +.+.f)]
    %6  [%6 (lower -.+.f) (lower -.+.+.f) (lower +.+.+.f)]
    %7  [%7 (lower -.+.f) (lower +.+.f)]
    %8  [%8 (lower -.+.f) (lower +.+.f)]
    ::
    ::  [9 b c] → [7 c' [2 [0 1] [0 b]]]
    ::  arm call: compute c to get core, pull arm at axis b
    ::
      %9
    =/  b  -.+.f
    =/  c  +.+.f
    [%7 (lower c) [%2 [%0 1] [%0 b]]]
    ::
    ::  [10 [b c] d] → [8 [d' c'] (make-edit b)]
    ::  axis edit: compute tree (d) and value (c), then
    ::  reconstruct tree with value spliced at axis b
    ::
      %10
    ::  b is the edit axis; Nock 10 only makes sense with an atom
    ::  axis, so cast explicitly. ;; crashes if `f` is malformed.
    ::
    =/  b=@  ;;(@ -.-.+.f)
    =/  c    +.-.+.f
    =/  d    +.+.f
    [%8 [(lower d) (lower c)] (make-edit b)]
    ::
    ::  [11 ...] — strip hints (semantically transparent)
    ::
      %11
    ?^  -.+.f
      (lower +.+.f)                                ::  dynamic: [11 [b c] d] → d'
    (lower +.+.f)                                  ::  static:  [11 b c] → c'
  ==
::
::  +make-edit: generate Nock 0-8 formula for #[ax value tree]
::
::  The lowered Nock 10 sets up subject [[tree value] s] via
::  Nock 8.  tree lands at axis 4, value at axis 5.
::
::  Algorithm: convert axis to a root-to-target path, walk it
::  to collect sibling subject-axes, then build the edit formula
::  inside-out (target back to root), combining the replacement
::  with each sibling at every level.
::
++  make-edit
  |=  ax=@
  ^-  *
  ?:  =(ax 0)  !!
  ?:  =(ax 1)  [%0 5]
  ::  axis → root-to-target path
  ::  %.n = head (even), %.y = tail (odd)
  ::
  =/  steps=(list ?)
    =|  s=(list ?)
    =/  n=@  ax
    |-
    ?:  =(n 1)  s
    ?:  =(0 (mod n 2))
      $(s [%.n s], n (div n 2))
    $(s [%.y s], n (div n 2))
  ::  walk path from root, record sibling subject-axes
  ::  prepend → result is target-to-root order
  ::
  =/  sibs=(list [? @])
    =|  acc=(list [? @])
    =/  cur=@  4
    |-
    ?~  steps  acc
    =/  h=@  (mul 2 cur)
    =/  t=@  +(h)
    ?:  i.steps
      ::  tail step: sibling is the head child
      $(acc [[%.y h] acc], cur t, steps t.steps)
    ::  head step: sibling is the tail child
    $(acc [[%.n t] acc], cur h, steps t.steps)
  ::  build edit formula inside-out
  ::
  =/  f=*  [%0 5]
  |-
  ?~  sibs  f
  ?:  -.i.sibs
    ::  came from tail: sibling on left, replacement on right
    $(f [[%0 +.i.sibs] f], sibs t.sibs)
  ::  came from head: replacement on left, sibling on right
  $(f [f [%0 +.i.sibs]], sibs t.sibs)
--
