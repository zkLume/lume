::  protocol/tests/test-intent-graft.hoon
::
::  Compile-time check that the family-5 intent-graft placeholder
::  types are well-formed. Runtime invocation of any %intent-* cause
::  arm crashes with %intent-graft-placeholder — that behavior is by
::  design and is covered by assert-crash below.
::
/+  *intent-graft
/+  *vesl-test
::
::  +new-state produces a structurally valid empty intent-state.
::
=/  s=intent-state  new-state
?>  (assert-eq intent-count.s 0)
?>  (assert-eq ~(wyt by intents.s) 0)
?>  (assert-eq ~(wyt by by-hull.s) 0)
::
::  The cause and effect unions are inhabited — building a sample of
::  each proves the type surface exists even though the placeholder
::  never returns any effect.
::
=/  c=intent-cause  [%intent-cancel id=`@`42]
?>  (assert-eq -.c %intent-cancel)
::
=/  e=intent-effect  [%intent-error msg=(crip "placeholder")]
?>  (assert-eq -.e %intent-error)
::
::  Every cause arm crashes loudly. Each assert-crash below proves
::  the placeholder is behaving as a placeholder and not as a silent
::  no-op.
::
?>  %-  assert-crash
    |.  (intent-poke new-state [%intent-declare hull=1 body=~ expires-at=~2026.5.1])
?>  %-  assert-crash
    |.  (intent-poke new-state [%intent-match id=1 proof=~])
?>  %-  assert-crash
    |.  (intent-poke new-state [%intent-cancel id=1])
?>  %-  assert-crash
    |.  (intent-poke new-state [%intent-expire id=1])
::
::  +intent-peek returns ~ so the host kernel's peek chain can fall
::  through; no canonical peek shape exists yet.
::
?>  (assert-eq (intent-peek new-state /intent/1) ~)
::
%pass
