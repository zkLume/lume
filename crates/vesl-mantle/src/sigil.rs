//! Sigil — Data Commitment (lightest tier)
//!
//! Commit data, get a root. No kernel boot required.
//! Pure math: tip5 Merkle tree construction and proof generation.

use nockchain_tip5_rs::{MerkleTree, ProofNode, Tip5Hash};

pub struct Sigil {
    tree: Option<MerkleTree>,
}

impl Sigil {
    /// Create a new Sigil committer.
    pub fn new() -> Self {
        Sigil { tree: None }
    }

    /// Commit a set of data chunks. Returns the Merkle root.
    /// Builds the tree internally and stores it for later proof generation.
    pub fn commit(&mut self, data: &[&[u8]]) -> Tip5Hash {
        let tree = MerkleTree::build(data);
        let root = tree.root();
        self.tree = Some(tree);
        root
    }

    /// Generate a Merkle proof for a specific leaf index.
    ///
    /// Panics if no tree has been committed or index is out of range.
    pub fn proof(&self, index: usize) -> Vec<ProofNode> {
        self.tree
            .as_ref()
            .expect("no tree committed — call commit() first")
            .proof(index)
    }

    /// Get the current root, or None if nothing committed yet.
    pub fn root(&self) -> Option<Tip5Hash> {
        self.tree.as_ref().map(|t| t.root())
    }
}

impl Default for Sigil {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nockchain_tip5_rs::verify_proof;

    #[test]
    fn commit_and_prove_single_leaf() {
        let mut sigil = Sigil::new();
        let data: &[&[u8]] = &[b"hello world"];
        let root = sigil.commit(data);

        assert!(sigil.root().is_some());
        assert_eq!(sigil.root().unwrap(), root);

        let proof = sigil.proof(0);
        assert!(verify_proof(b"hello world", &proof, &root));
    }

    #[test]
    fn commit_and_prove_multiple_leaves() {
        let mut sigil = Sigil::new();
        let chunks: Vec<&[u8]> = vec![
            b"alpha",
            b"bravo",
            b"charlie",
            b"delta",
        ];
        let root = sigil.commit(&chunks);

        for (i, chunk) in chunks.iter().enumerate() {
            let proof = sigil.proof(i);
            assert!(verify_proof(chunk, &proof, &root), "failed at leaf {i}");
        }
    }

    #[test]
    fn tampered_data_fails_verification() {
        let mut sigil = Sigil::new();
        let root = sigil.commit(&[b"real data"]);
        let proof = sigil.proof(0);

        assert!(!verify_proof(b"fake data", &proof, &root));
    }

    #[test]
    fn root_is_none_before_commit() {
        let sigil = Sigil::new();
        assert!(sigil.root().is_none());
    }

    #[test]
    fn recommit_replaces_tree() {
        let mut sigil = Sigil::new();
        let root1 = sigil.commit(&[b"v1"]);
        let root2 = sigil.commit(&[b"v2"]);
        assert_ne!(root1, root2);
        assert_eq!(sigil.root().unwrap(), root2);
    }
}
