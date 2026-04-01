# nockchain-tip5-rs

> ~100 constraints per hash. SHA-256 needs ~30,000. Do the math.

Standalone tip5 Merkle tree for Nockchain. Hash functions, tree
construction, proof generation, proof verification, and test vectors
you can actually read.

## Why tip5

tip5 is an algebraic hash function designed for zero-knowledge circuits.
Where SHA-256 chews through tens of thousands of arithmetic constraints
per invocation, tip5 does it in roughly a hundred. This isn't an academic
curiosity — it's the difference between "your STARK proof takes 3 seconds"
and "your STARK proof takes 3 minutes."

Nockchain uses tip5 natively. If you're building on Nockchain and you're
not using tip5, you're carrying SHA-256's weight for no reason.

## Quick start

```rust
use nockchain_tip5_rs::*;

// Build a Merkle tree from document chunks
let leaves: Vec<&[u8]> = vec![
    b"patient record: blood-type A+",
    b"trading algo: momentum signal",
    b"NDA clause 4: non-compete",
    b"the AI read this secret",
];
let tree = MerkleTree::build(&leaves);
let root = tree.root();

// Generate an inclusion proof for leaf 0
let proof = tree.proof(0);

// Verify it (or don't — but the chain will)
assert!(verify_proof(leaves[0], &proof, &root));

// Tampered data fails. Always.
assert!(!verify_proof(b"the AI wrote this secret", &proof, &root));
```

## API

### Hash primitives

| Function | Description |
|----------|-------------|
| `hash_leaf(data)` | tip5 hash of raw bytes — splits into 7-byte Goldilocks field elements, prepends count, sponge absorb |
| `hash_pair(left, right)` | tip5 pair hash — concatenates two 5-limb digests into 10 belts, fixed-rate sponge |
| `verify_proof(data, proof, root)` | Walk a Merkle proof from leaf to root. Returns bool. No exceptions, no panics, no drama. |

### Tree construction

```rust
let tree = MerkleTree::build(&leaves);  // panics on empty input, as it should
let root = tree.root();                  // the one hash to rule them all
let proof = tree.proof(2);               // inclusion proof for leaf at index 2
let n = tree.leaf_count();               // number of original leaves
```

Odd-count levels are padded by duplicating the last node. This is
standard Merkle convention and matches the Hoon-side implementation
in `vesl-logic.hoon`.

### Cross-VM encoding

```rust
let atom_bytes = tip5_to_atom_le_bytes(&hash);
```

Converts a `[u64; 5]` digest to the flat atom byte representation used
in Nock nouns. Computes the base-p polynomial (`a + b*P + c*P^2 + ...`
where P = Goldilocks prime). Byte-identical to Hoon's
`digest-to-atom:tip5`. This is the bridge between Rust and the ZK
circuit.

### Types

```rust
pub type Tip5Hash = [u64; 5];  // 5 Goldilocks field elements

pub struct ProofNode {
    pub hash: Tip5Hash,        // sibling hash at this level
    pub side: bool,            // true = sibling is LEFT, false = RIGHT
}
```

Enable the `serde` feature for `Serialize`/`Deserialize` on `ProofNode`:

```toml
nockchain-tip5-rs = { version = "0.1", features = ["serde"] }
```

## Side convention

This trips people up. Here's the deal:

```
side = true  (%.y)  ->  sibling is LEFT   ->  hash_pair(sibling, current)
side = false (%.n)  ->  sibling is RIGHT  ->  hash_pair(current, sibling)
```

`hash_pair` is **non-commutative**. `hash_pair(a, b) != hash_pair(b, a)`.
This is not a bug. This is what prevents path-swap attacks. If someone
flips a side flag in your proof, the hash cascade produces a completely
different root. This is tested. This is proven. This is the point.

## Cross-VM alignment

This crate is the Rust mirror of Hoon's `vesl-logic.hoon`. Both sides:

1. Split atoms into 7-byte chunks (each < 2^56 < Goldilocks prime)
2. Prepend the chunk count as a belt
3. Feed to the same tip5 sponge

Same bytes in, same digest out. If this invariant ever breaks, the ZK
circuit can't verify Rust-built trees. We test for this. You should too.

## Test vectors

The test suite includes the "enterprise scenario" — 4 leaves of
sensitive data (medical records, trading algos, NDAs) used across the
entire Vesl test matrix. These vectors are stable across versions and
match the Hoon-side red-team tests byte-for-byte.

```rust
cargo test  // 14 tests: structure, determinism, verification,
            // tamper rejection, non-commutativity, edge cases
```

## Who made this

Extracted from [Vesl](https://github.com/zkVesl/vesl). Previously,
the only way to get tip5 in Rust was to depend on `nockchain-math` from
the monorepo and figure out the rest yourself. Now you don't have to.

You're welcome. `~`
