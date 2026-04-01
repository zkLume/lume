# vesl-test

> The Nockchain ecosystem has zero testing infrastructure. This is the first.

Compile-time testing patterns for Hoon. No runtime. No test runner.
No framework. Just assertions that crash the compiler when they fail.
If it compiles, it passes. If it doesn't, you know exactly why.

## Why compile-time testing

Hoon doesn't have `pytest`. It doesn't have `jest`. It doesn't have
`cargo test`. What it *does* have is a compiler that evaluates
arbitrary expressions and a type system that will fight you for sport.

We turned this into a testing strategy:

1. Write assertions as Hoon expressions
2. Compile with `hoonc --arbitrary`
3. If compilation succeeds, every assertion passed
4. If compilation fails, the crash trace tells you which one

No test harness. No assertion library with 47 methods. No mocking.
Just math that either works or doesn't.

## Install

```hoon
/+  *vesl-test
```

That's it. The `*` glob-imports all arms into your namespace.

## Assertions

### Equality

```hoon
::  These crash the compiler if they fail. That's the point.
?>  (assert-eq (add 2 2) 4)
?>  (assert-neq 'alice' 'bob')
```

### Expected crashes

The `mule` pattern. If you're testing that bad input crashes your gate,
wrap the call in a `|.` trap and hand it to `assert-crash`:

```hoon
::  This SHOULD crash. If it doesn't, something is very wrong.
?>  (assert-crash |.((settle-note bad-note bad-manifest root)))

::  This should NOT crash.
?>  (assert-ok |.((verify-chunk valid-dat proof root)))
```

### Hash comparison

When you're debugging cross-runtime alignment issues and need to see
*which* hashes don't match:

```hoon
?>  (assert-hash-eq (hash-leaf 'data') expected-hash)
?>  (assert-hash-neq (hash-leaf 'a') (hash-leaf 'b'))
```

On failure, both hash values appear in the crash trace. You're welcome.

### Boolean gates

For gates that return `?` (loobean):

```hoon
?>  (assert-flag (verify-chunk dat proof root))
?>  (assert-not-flag (verify-chunk tampered proof root))
```

## Full example

```hoon
/-  *vesl
/+  *vesl-logic
/+  *vesl-test
::
=/  h0  (hash-leaf 'alpha')
=/  h1  (hash-leaf 'bravo')
=/  root  (hash-pair h0 h1)
=/  proof=(list [hash=@ side=?])
  ~[[hash=h1 side=%.n]]
::
::  Valid proof verifies
?>  (assert-flag (verify-chunk 'alpha' proof root))
::
::  Tampered data fails
?>  (assert-not-flag (verify-chunk 'TAMPERED' proof root))
::
::  Different inputs produce different hashes
?>  (assert-hash-neq h0 h1)
::
%pass
```

Compile:

```bash
hoonc --new --arbitrary tests/my-test.hoon hoon/
```

If you see `build succeeded` — every assertion passed. Ship it.

## The full arsenal

| Arm | Asserts | On failure |
|-----|---------|------------|
| `assert-eq` | `a` equals `b` | Crash with `'assert-eq: values not equal'` |
| `assert-neq` | `a` does not equal `b` | Crash with `'assert-neq: values are equal'` |
| `assert-crash` | Trap crashes | Crash with `'computation succeeded (expected crash)'` |
| `assert-ok` | Trap succeeds | Crash with `'computation crashed (expected success)'` |
| `assert-hash-eq` | Hash atoms equal | Crash with both hash values in trace |
| `assert-hash-neq` | Hash atoms differ | Crash with the duplicate hash in trace |
| `assert-flag` | Flag is `%.y` | Crash with `'flag is %.n'` |
| `assert-not-flag` | Flag is `%.n` | Crash with `'flag is %.y'` |

## Philosophy

Tests should be boring. Infrastructure should be invisible. If you're
spending more time configuring your test framework than writing tests,
the framework has failed.

Eight arms. Zero configuration. Compiler does the rest.

```
::  the only test framework small enough
::  to fit in a Nock subject
```

Part of [Vesl](https://github.com/zkVesl/vesl). `~`
