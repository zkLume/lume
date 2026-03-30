::  tests/verify-retrieval.hoon: Merkle verification test harness
::
::  Builds a 4-leaf tree with real-world enterprise chunks,
::  asserts correct inclusion proof and rejects tampered data.
::  Compilation success = all assertions passed.
::
/-  *vesl
/+  *vesl-logic
::
::  Build 4-leaf Merkle tree — enterprise scenario chunks
::
::          root
::         /    \
::       h01    h23
::       / \    / \
::      h0  h1 h2  h3
::
=/  h0  (hash-leaf 'The AI read this secret.')
=/  h1  (hash-leaf 'Patient record: blood-type A+')
=/  h2  (hash-leaf 'Trading algo: momentum signal')
=/  h3  (hash-leaf 'NDA clause 4: non-compete')
=/  h01  (hash-pair h0 h1)
=/  h23  (hash-pair h2 h3)
=/  root  (hash-pair h01 h23)
::
::  Proof for leaf 0 ('The AI read this secret.'):
::    level 1: sibling h1 is RIGHT  -> side=%.n
::    level 2: sibling h23 is RIGHT -> side=%.n
::
=/  proof=(list [hash=@ side=?])
  ~[[hash=h1 side=%.n] [hash=h23 side=%.n]]
::
::  Test 1: correct chunk + correct proof -> %.y
::
?>  (verify-chunk 'The AI read this secret.' proof root)
::
::  Test 2: tampered chunk + correct proof -> %.n
::
?<  (verify-chunk 'The AI modified this secret.' proof root)
::
%pass
