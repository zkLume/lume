//! Merkle tree engine — mathematical mirror of protocol/lib/lume-logic.hoon.
//!
//! # tip5 Hash Alignment Proof
//!
//! Hoon's `hash-leaf` / `hash-pair` (via `zeke.hoon`) and Rust's tip5
//! functions (via `nockchain-math`) must produce identical digests for
//! the ZK-circuit to verify Rust-built trees.
//!
//! ## Leaf hashing (`hash_leaf` ↔ `+hash-leaf`)
//!
//! Both sides split the atom's LE bytes into 7-byte chunks (each < 2^56 <
//! Goldilocks prime), prepend the chunk count, and feed to `hash_varlen`.
//!
//! Hoon:  `(end [3 7] a)` / `(rsh [3 7] a)` loop → belt list
//! Rust:  `bytes.chunks(7)` → Vec<Belt>
//! Same chunking → same belts → same tip5 sponge input → identical digest.
//!
//! ## Pair hashing (`hash_pair` ↔ `+hash-pair`)
//!
//! Both sides concatenate two 5-limb digests into 10 belts and feed to
//! `hash_10` (tip5 fixed-rate sponge).
//!
//! Hoon:  `(hash-ten-cell:tip5 [ld rd])` after `atom-to-digest` conversion.
//! Rust:  `hash_10(&mut [l0..l4, r0..r4])`.
//! Same 10 belts → same permutation → identical digest.

use nockchain_math::belt::Belt;
use nockchain_math::tip5::hash::{hash_10, hash_varlen};

use crate::types::{ProofNode, Tip5Hash};

/// Split LE atom bytes into 7-byte chunks, each a valid Goldilocks field element.
///
/// Mirrors Hoon's `+split-to-belts`: `(end [3 7] a)` / `(rsh [3 7] a)` loop.
/// 7 bytes = 56 bits → max value 2^56 - 1 ≈ 7.2e16 < PRIME ≈ 1.8e19.
fn atom_bytes_to_belts(bytes: &[u8]) -> Vec<Belt> {
    // Trim trailing zeros — Hoon atoms have no leading LE zeros.
    let len = bytes.iter().rposition(|&b| b != 0).map_or(0, |p| p + 1);
    if len == 0 {
        return vec![Belt(0)];
    }
    let bytes = &bytes[..len];
    let mut belts = Vec::with_capacity((len + 6) / 7);
    for chunk in bytes.chunks(7) {
        let mut val: u64 = 0;
        for (i, &b) in chunk.iter().enumerate() {
            val |= (b as u64) << (i * 8);
        }
        belts.push(Belt(val));
    }
    belts
}

/// tip5 hash of raw leaf data.
/// Mirror of `+hash-leaf`: split → belts → prepend count → hash_varlen → digest.
pub fn hash_leaf(data: &[u8]) -> Tip5Hash {
    let belts = atom_bytes_to_belts(data);
    let count = belts.len() as u64;
    let mut input: Vec<Belt> = Vec::with_capacity(1 + belts.len());
    input.push(Belt(count));
    input.extend(belts);
    hash_varlen(&mut input)
}

/// tip5 pair hash of two digests via fixed-rate sponge (10 belts).
/// Mirror of `+hash-pair`: `(hash-ten-cell:tip5 [ld rd])`.
///
/// Byte layout: `[l0, l1, l2, l3, l4, r0, r1, r2, r3, r4]`
pub fn hash_pair(l: &Tip5Hash, r: &Tip5Hash) -> Tip5Hash {
    let mut input: Vec<Belt> = l.iter().chain(r.iter()).map(|&v| Belt(v)).collect();
    hash_10(&mut input)
}

/// Verify a Merkle proof — Rust-side mirror of Hoon's `+verify-chunk`.
///
/// Walks the proof path from leaf to root, applying the same side convention:
///   side=true  (%.y) → sibling is LEFT  → hash_pair(sibling, current)
///   side=false (%.n) → sibling is RIGHT → hash_pair(current, sibling)
pub fn verify_proof(leaf_data: &[u8], proof: &[ProofNode], expected_root: &Tip5Hash) -> bool {
    let mut cur = hash_leaf(leaf_data);

    for node in proof {
        cur = if node.side {
            hash_pair(&node.hash, &cur)
        } else {
            hash_pair(&cur, &node.hash)
        };
    }

    cur == *expected_root
}

/// A complete Merkle tree built from leaf data.
pub struct MerkleTree {
    /// Nodes stored level-by-level: levels[0] = leaf hashes, last = root.
    levels: Vec<Vec<Tip5Hash>>,
}

impl MerkleTree {
    /// Build a Merkle tree from raw leaf byte slices.
    /// Pads odd-count levels by duplicating the last node.
    pub fn build(leaves: &[&[u8]]) -> Self {
        assert!(!leaves.is_empty(), "cannot build tree from zero leaves");

        let mut current: Vec<Tip5Hash> = leaves.iter().map(|l| hash_leaf(l)).collect();
        let mut levels = vec![current.clone()];

        while current.len() > 1 {
            // Pad odd levels by duplicating the last hash
            if current.len() % 2 != 0 {
                let last = *current.last().unwrap();
                current.push(last);
            }

            let next: Vec<Tip5Hash> = current
                .chunks(2)
                .map(|pair| hash_pair(&pair[0], &pair[1]))
                .collect();

            levels.push(next.clone());
            current = next;
        }

        MerkleTree { levels }
    }

    /// The Merkle root hash.
    pub fn root(&self) -> Tip5Hash {
        *self.levels.last().unwrap().first().unwrap()
    }

    /// Generate the proof path for the leaf at `index`.
    ///
    /// Side convention (mirrors Hoon's `verify-chunk`):
    ///   Even index (left child)  → sibling is RIGHT → side=false
    ///   Odd index  (right child) → sibling is LEFT  → side=true
    pub fn proof(&self, index: usize) -> Vec<ProofNode> {
        assert!(index < self.levels[0].len(), "leaf index out of bounds");

        let mut path = Vec::new();
        let mut idx = index;

        for level in &self.levels[..self.levels.len() - 1] {
            let sibling_idx = if idx % 2 == 0 { idx + 1 } else { idx - 1 };

            // If sibling doesn't exist (odd count, last element), it was
            // padded with a duplicate — use our own hash as the sibling.
            let sibling_hash = if sibling_idx < level.len() {
                level[sibling_idx]
            } else {
                level[idx]
            };

            path.push(ProofNode {
                hash: sibling_hash,
                side: idx % 2 == 1, // odd = right child = sibling is LEFT = true
            });

            idx /= 2;
        }

        path
    }
}

/// Format a tip5 hash for display: show limbs in decimal.
pub fn format_tip5(hash: &Tip5Hash) -> String {
    format!(
        "[{:016x}.{:016x}.{:016x}.{:016x}.{:016x}]",
        hash[0], hash[1], hash[2], hash[3], hash[4]
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Enterprise scenario leaves — matches the Hoon red-team test data.
    fn enterprise_leaves() -> Vec<&'static [u8]> {
        vec![
            b"The AI read this secret.",
            b"Patient record: blood-type A+",
            b"Trading algo: momentum signal",
            b"NDA clause 4: non-compete",
        ]
    }

    #[test]
    fn atom_bytes_to_belts_small() {
        // "alpha" = [97, 108, 112, 104, 97] — 5 bytes, fits in one 7-byte chunk
        let belts = atom_bytes_to_belts(b"alpha");
        assert_eq!(belts.len(), 1);
        let expected: u64 = 97 + 108 * 256 + 112 * 65536 + 104 * 16777216 + 97 * 4294967296;
        assert_eq!(belts[0].0, expected);
    }

    #[test]
    fn atom_bytes_to_belts_multi_chunk() {
        // 15 bytes → 3 chunks (7 + 7 + 1)
        let data = b"0123456789abcde";
        let belts = atom_bytes_to_belts(data);
        assert_eq!(belts.len(), 3); // ceil(15/7) = 3
        // First chunk: bytes[0..7]
        let mut expected0: u64 = 0;
        for (i, &b) in data[..7].iter().enumerate() {
            expected0 |= (b as u64) << (i * 8);
        }
        assert_eq!(belts[0].0, expected0);
    }

    #[test]
    fn atom_bytes_to_belts_zero() {
        let belts = atom_bytes_to_belts(&[]);
        assert_eq!(belts, vec![Belt(0)]);
        let belts2 = atom_bytes_to_belts(&[0, 0, 0]);
        assert_eq!(belts2, vec![Belt(0)]);
    }

    #[test]
    fn build_4_leaf_tree_structure() {
        let tree = MerkleTree::build(&enterprise_leaves());

        // 3 levels: 4 leaves → 2 intermediate → 1 root
        assert_eq!(tree.levels.len(), 3);
        assert_eq!(tree.levels[0].len(), 4);
        assert_eq!(tree.levels[1].len(), 2);
        assert_eq!(tree.levels[2].len(), 1);
    }

    #[test]
    fn tree_is_deterministic() {
        let leaves = enterprise_leaves();
        let root1 = MerkleTree::build(&leaves).root();
        let root2 = MerkleTree::build(&leaves).root();
        assert_eq!(root1, root2);
    }

    #[test]
    fn verify_all_leaves() {
        let leaves = enterprise_leaves();
        let tree = MerkleTree::build(&leaves);
        let root = tree.root();

        for (i, leaf) in leaves.iter().enumerate() {
            let proof = tree.proof(i);
            assert!(
                verify_proof(leaf, &proof, &root),
                "valid proof for leaf {} rejected",
                i
            );
        }
    }

    #[test]
    fn reject_tampered_leaf() {
        let leaves = enterprise_leaves();
        let tree = MerkleTree::build(&leaves);
        let root = tree.root();
        let proof = tree.proof(0);

        assert!(
            !verify_proof(b"TAMPERED DATA", &proof, &root),
            "tampered leaf should not verify"
        );
    }

    #[test]
    fn reject_wrong_root() {
        let leaves = enterprise_leaves();
        let tree = MerkleTree::build(&leaves);
        let proof = tree.proof(0);
        let wrong_root = [0xFFu64; 5];

        assert!(
            !verify_proof(leaves[0], &proof, &wrong_root),
            "wrong root should not verify"
        );
    }

    #[test]
    fn hash_pair_is_non_commutative() {
        // Mirrors Red Team Attack 2 (Path Swap): hash(a, b) != hash(b, a)
        let a = hash_leaf(b"left");
        let b = hash_leaf(b"right");
        assert_ne!(
            hash_pair(&a, &b),
            hash_pair(&b, &a),
            "hash_pair must be non-commutative"
        );
    }

    #[test]
    fn single_leaf_tree() {
        let leaves: Vec<&[u8]> = vec![b"only leaf"];
        let tree = MerkleTree::build(&leaves);

        assert_eq!(tree.levels.len(), 1);
        assert_eq!(tree.root(), hash_leaf(b"only leaf"));

        let proof = tree.proof(0);
        assert!(proof.is_empty());
        assert!(verify_proof(b"only leaf", &proof, &tree.root()));
    }
}
