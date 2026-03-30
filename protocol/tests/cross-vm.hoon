::  protocol/tests/cross-vm.hoon: Cross-VM alignment proof
::
::  Uses the EXACT same data as the Rust Hull's test scenario:
::    - Same 4 chunks (Q3 revenue, risk exposure, board, SOC2)
::    - Same query ("Summarize Q3 financial position")
::    - Same retrieved indices (0, 1) with score=950.000
::    - Same prompt construction (query + \n + chunk0 + \n + chunk1)
::
::  If this compiles, the Hoon ZK-circuit correctly processes
::  data matching the Rust Hull's output.
::
/-  *vesl
/+  *vesl-logic
/+  *vesl-entrypoint
::
::  ============================================
::  EXACT Rust Hull data — same as hull/src/noun_builder.rs
::  ============================================
::
=/  h0  (hash-leaf 'Q3 revenue: $4.2M ARR, 18% QoQ growth')
=/  h1  (hash-leaf 'Risk exposure: $800K in variable-rate instruments')
=/  h2  (hash-leaf 'Board approved Series B at $45M pre-money')
=/  h3  (hash-leaf 'SOC2 Type II audit scheduled for Q4')
=/  h01  (hash-pair h0 h1)
=/  h23  (hash-pair h2 h3)
=/  root  (hash-pair h01 h23)
::
::  Retrieved chunks 0 and 1 with proofs
::
=/  chunk0  [id=0 dat='Q3 revenue: $4.2M ARR, 18% QoQ growth']
=/  chunk1  [id=1 dat='Risk exposure: $800K in variable-rate instruments']
::
::  Proof for leaf 0 (even=left child):
::    level 0: sibling=h1 side=%.n (sibling is RIGHT)
::    level 1: sibling=h23 side=%.n (sibling is RIGHT)
::
=/  proof0=(list [hash=@ side=?])
  ~[[hash=h1 side=%.n] [hash=h23 side=%.n]]
::
::  Proof for leaf 1 (odd=right child):
::    level 0: sibling=h0 side=%.y (sibling is LEFT)
::    level 1: sibling=h23 side=%.n (sibling is RIGHT)
::
=/  proof1=(list [hash=@ side=?])
  ~[[hash=h0 side=%.y] [hash=h23 side=%.n]]
::
=/  results=(list [chunk=[id=@ dat=@t] proof=(list [hash=@ side=?]) score=@ud])
  ~[[chunk=chunk0 proof=proof0 score=950.000] [chunk=chunk1 proof=proof1 score=950.000]]
::
::  Prompt: query + \n + chunk0.dat + \n + chunk1.dat
::
=/  query=@t  'Summarize Q3 financial position'
=/  sep  10
=/  s1  (cat 3 query sep)
=/  s2  (cat 3 s1 'Q3 revenue: $4.2M ARR, 18% QoQ growth')
=/  s3  (cat 3 s2 sep)
=/  valid-prompt=@t  `@t`(cat 3 s3 'Risk exposure: $800K in variable-rate instruments')
::
::  Output (mock LLM response — matches Rust run_inference)
::
=/  output=@t  'Based on the provided documents: Q3 revenue: $4.2M ARR, 18% QoQ growth | Risk exposure: $800K in variable-rate instruments The analysis indicates positive growth trajectory.'
::
=/  valid-mani
  [query=query results=results prompt=valid-prompt output=output]
::
=/  pending-note  [id=42 hull=7 root=root state=[%pending ~]]
::
::  ============================================
::  TEST 1: Direct settlement (no jam/cue boundary)
::  ============================================
::
=/  direct-result  (settle-note pending-note valid-mani root)
?>  =(state.direct-result [%settled ~])
?>  =(id.direct-result 42)
?>  =(hull.direct-result 7)
::
::  ============================================
::  TEST 2: Full ABI boundary (jam → cue → ;; → settle)
::  ============================================
::
=/  payload=@  (jam [pending-note valid-mani root])
=/  abi-result  (vesl-entrypoint payload)
?>  =(state.abi-result [%settled ~])
?>  =(id.abi-result 42)
?>  =(hull.abi-result 7)
::
::  ============================================
::  TEST 3: Output the jammed payload for comparison with Rust
::  ============================================
::
::  The payload atom can be compared byte-for-byte with
::  hull/tests/test_payload.jam to prove cross-VM alignment.
::
payload
