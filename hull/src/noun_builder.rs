//! Nock noun construction and jam serialization for the Vesl settlement payload.
//!
//! Uses `nock-noun-rs` for generic noun building and `nockchain-tip5-rs` for
//! hash encoding. This module adds Vesl-specific structure builders matching
//! `protocol/lib/vesl-entrypoint.hoon:+$settlement-payload`.

use nock_noun_rs::{
    make_atom, make_atom_in, make_cord, make_loobean,
    new_stack, Cell, D, NounSlab, NockStack, Noun, NounAllocator, T,
    jam,
};
use nockchain_tip5_rs::tip5_to_atom_le_bytes;

#[cfg(test)]
use crate::merkle::MerkleTree;
use crate::types::*;

// ---------------------------------------------------------------------------
// Vesl-specific atom builders
// ---------------------------------------------------------------------------

/// Convert a tip5 hash `[u64; 5]` to a Nock atom.
///
/// Uses base-p polynomial encoding: `a + b*P + c*P^2 + d*P^3 + e*P^4`
/// matching Hoon's `digest-to-atom:tip5`.
fn hash_to_noun(stack: &mut NockStack, hash: &Tip5Hash) -> Noun {
    let le_bytes = tip5_to_atom_le_bytes(hash);
    make_atom(stack, &le_bytes)
}

/// Convert a tip5 hash to a Nock atom using any allocator.
fn hash_to_noun_generic(alloc: &mut impl NounAllocator, hash: &Tip5Hash) -> Noun {
    let le_bytes = tip5_to_atom_le_bytes(hash);
    make_atom_in(alloc, &le_bytes)
}

// ---------------------------------------------------------------------------
// Structure Builders — mirror protocol/sur/vesl.hoon
// ---------------------------------------------------------------------------

/// `+$proof-node  [hash=@ side=?]`
fn proof_node_to_noun(stack: &mut NockStack, node: &ProofNode) -> Noun {
    let h = hash_to_noun(stack, &node.hash);
    let s = make_loobean(node.side);
    T(stack, &[h, s])
}

/// `(list proof-node)` -> null-terminated right-leaning cell tree.
fn proof_list_to_noun(stack: &mut NockStack, proof: &[ProofNode]) -> Noun {
    let mut list = D(0); // null terminator
    for node in proof.iter().rev() {
        let item = proof_node_to_noun(stack, node);
        list = Cell::new(stack, item, list).as_noun();
    }
    list
}

/// `+$chunk  [id=chunk-id dat=@t]`
fn chunk_to_noun(stack: &mut NockStack, chunk: &Chunk) -> Noun {
    let id = D(chunk.id);
    let dat = make_cord(stack, &chunk.dat);
    T(stack, &[id, dat])
}

/// `+$retrieval  [=chunk proof=merkle-proof score=@ud]`
fn retrieval_to_noun(stack: &mut NockStack, r: &Retrieval) -> Noun {
    let c = chunk_to_noun(stack, &r.chunk);
    let p = proof_list_to_noun(stack, &r.proof);
    let s = D(r.score);
    T(stack, &[c, p, s])
}

/// `(list retrieval)` -> null-terminated right-leaning.
fn retrieval_list_to_noun(stack: &mut NockStack, results: &[Retrieval]) -> Noun {
    let mut list = D(0);
    for r in results.iter().rev() {
        let item = retrieval_to_noun(stack, r);
        list = Cell::new(stack, item, list).as_noun();
    }
    list
}

/// `+$manifest  [query=@t results=(list retrieval) prompt=@t output=@t]`
fn manifest_to_noun(stack: &mut NockStack, m: &Manifest) -> Noun {
    let query = make_cord(stack, &m.query);
    let results = retrieval_list_to_noun(stack, &m.results);
    let prompt = make_cord(stack, &m.prompt);
    let output = make_cord(stack, &m.output);
    T(stack, &[query, results, prompt, output])
}

/// `note=[id=@ hull=@ root=@ state=[%pending ~]]`
///
/// In Nock: `[id [hull [root [%pending 0]]]]`
fn pending_note_to_noun(stack: &mut NockStack, note: &Note) -> Noun {
    assert!(
        matches!(note.state, NoteState::Pending),
        "settlement payload requires %pending note"
    );
    let id = D(note.id);
    let hull = D(note.hull);
    let root = hash_to_noun(stack, &note.root);
    let tag = make_atom(stack, b"pending");
    let state = Cell::new(stack, tag, D(0)).as_noun(); // [%pending ~]
    T(stack, &[id, hull, root, state])
}

// ---------------------------------------------------------------------------
// Settlement Payload — the ABI boundary
// ---------------------------------------------------------------------------

/// Build the complete settlement payload noun.
///
/// Matches `+$settlement-payload` from `vesl-entrypoint.hoon`:
/// ```text
/// [note=[id=@ hull=@ root=@ state=[%pending ~]]
///  mani=[query=@t results=(list ...) prompt=@t output=@t]
///  expected-root=@]
/// ```
pub fn build_settlement_payload(
    stack: &mut NockStack,
    note: &Note,
    manifest: &Manifest,
    expected_root: &Tip5Hash,
) -> Noun {
    let note_noun = pending_note_to_noun(stack, note);
    let mani_noun = manifest_to_noun(stack, manifest);
    let root_noun = hash_to_noun(stack, expected_root);
    T(stack, &[note_noun, mani_noun, root_noun])
}

/// Full pipeline: build settlement noun -> jam -> bytes.
#[cfg(test)]
pub fn serialize_settlement(
    stack: &mut NockStack,
    note: &Note,
    manifest: &Manifest,
    expected_root: &Tip5Hash,
) -> Vec<u8> {
    let payload = build_settlement_payload(stack, note, manifest, expected_root);
    nock_noun_rs::jam_to_bytes(stack, payload)
}

// ---------------------------------------------------------------------------
// NounSlab-based poke builders — for NockApp kernel interaction
// ---------------------------------------------------------------------------

/// Build a `%settle` poke cause in a NounSlab.
///
/// Constructs the settlement payload noun, jams it, then wraps as
/// `[%settle jammed-atom]` matching the kernel's `+$cause`.
pub fn build_settle_poke(
    note: &Note,
    manifest: &Manifest,
    expected_root: &Tip5Hash,
) -> NounSlab {
    let mut stack = new_stack();
    let payload = build_settlement_payload(&mut stack, note, manifest, expected_root);
    let jammed = jam(&mut stack, payload);
    let jam_bytes = {
        let bytes = jammed.as_ne_bytes();
        let len = bytes.iter().rposition(|&b| b != 0).map_or(0, |pos| pos + 1);
        bytes[..len].to_vec()
    };

    let mut slab = NounSlab::new();
    let tag = make_atom_in(&mut slab, b"settle");
    let payload_atom = make_atom_in(&mut slab, &jam_bytes);
    let cause = T(&mut slab, &[tag, payload_atom]);
    slab.set_root(cause);
    slab
}

/// Build a `%prove` poke cause in a NounSlab.
///
/// Same payload as `%settle` but tagged `%prove`.
pub fn build_prove_poke(
    note: &Note,
    manifest: &Manifest,
    expected_root: &Tip5Hash,
) -> NounSlab {
    let mut stack = new_stack();
    let payload = build_settlement_payload(&mut stack, note, manifest, expected_root);
    let jammed = jam(&mut stack, payload);
    let jam_bytes = {
        let bytes = jammed.as_ne_bytes();
        let len = bytes.iter().rposition(|&b| b != 0).map_or(0, |pos| pos + 1);
        bytes[..len].to_vec()
    };

    let mut slab = NounSlab::new();
    let tag = make_atom_in(&mut slab, b"prove");
    let payload_atom = make_atom_in(&mut slab, &jam_bytes);
    let cause = T(&mut slab, &[tag, payload_atom]);
    slab.set_root(cause);
    slab
}

/// Build a `%register` poke cause in a NounSlab.
///
/// Constructs `[%register hull=@ root=@]`.
pub fn build_register_poke(hull_id: u64, root: &Tip5Hash) -> NounSlab {
    let mut slab = NounSlab::new();
    let tag = make_atom_in(&mut slab, b"register");
    let id = D(hull_id);
    let root_noun = hash_to_noun_generic(&mut slab, root);
    let cause = T(&mut slab, &[tag, id, root_noun]);
    slab.set_root(cause);
    slab
}

// ---------------------------------------------------------------------------
// Helper: build the full scenario (for testing and main pipeline)
// ---------------------------------------------------------------------------

/// Build a complete Hedge Fund scenario: 4 chunks, retrieve 0 and 1.
/// Returns (note, manifest, root) ready for serialization.
#[cfg(test)]
pub fn build_hedge_fund_scenario() -> (Note, Manifest, Tip5Hash) {
    let chunks = vec![
        Chunk {
            id: 0,
            dat: "Q3 revenue: $4.2M ARR, 18% QoQ growth".into(),
        },
        Chunk {
            id: 1,
            dat: "Risk exposure: $800K in variable-rate instruments".into(),
        },
        Chunk {
            id: 2,
            dat: "Board approved Series B at $45M pre-money".into(),
        },
        Chunk {
            id: 3,
            dat: "SOC2 Type II audit scheduled for Q4".into(),
        },
    ];

    let leaf_data: Vec<&[u8]> = chunks.iter().map(|c| c.dat.as_bytes()).collect();
    let tree = MerkleTree::build(&leaf_data);
    let root = tree.root();

    let query = "Summarize Q3 financial position";
    let retrieved_indices = [0usize, 1];

    let retrievals: Vec<Retrieval> = retrieved_indices
        .iter()
        .map(|&i| Retrieval {
            chunk: chunks[i].clone(),
            proof: tree.proof(i),
            score: 950_000,
        })
        .collect();

    let mut prompt = query.to_string();
    for &i in &retrieved_indices {
        prompt.push('\n');
        prompt.push_str(&chunks[i].dat);
    }

    let output = format!(
        "Based on the provided documents: {} The analysis indicates positive growth trajectory.",
        retrieved_indices
            .iter()
            .map(|&i| chunks[i].dat.as_str())
            .collect::<Vec<_>>()
            .join(" | ")
    );

    let manifest = Manifest {
        query: query.to_string(),
        results: retrievals,
        prompt,
        output,
    };

    let note = Note {
        id: 42,
        hull: 7,
        root,
        state: NoteState::Pending,
    };

    (note, manifest, root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;
    use nock_noun_rs::cue;

    #[test]
    fn jam_and_write_payload() {
        let (note, manifest, root) = build_hedge_fund_scenario();
        let mut stack = new_stack();

        let jam_bytes = serialize_settlement(&mut stack, &note, &manifest, &root);

        assert!(!jam_bytes.is_empty(), "jammed payload must not be empty");
        println!("JAM payload: {} bytes", jam_bytes.len());
        println!(
            "First 32 bytes: {}",
            hex::encode(&jam_bytes[..32.min(jam_bytes.len())])
        );

        let test_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
        fs::create_dir_all(&test_dir).expect("create tests dir");
        let jam_path = test_dir.join("test_payload.jam");
        fs::write(&jam_path, &jam_bytes).expect("write jam file");
        println!("Wrote {} bytes to {}", jam_bytes.len(), jam_path.display());

        let read_back = fs::read(&jam_path).expect("read jam file");
        assert_eq!(read_back, jam_bytes, "file content must match");
    }

    #[test]
    fn jam_cue_round_trip() {
        let (note, manifest, root) = build_hedge_fund_scenario();
        let mut stack = new_stack();

        let payload = build_settlement_payload(&mut stack, &note, &manifest, &root);
        let jammed = jam(&mut stack, payload);
        let cued = cue(&mut stack, jammed).expect("cue must succeed on jammed payload");

        assert!(cued.is_cell(), "cued noun must be a cell");

        let outer = cued.as_cell().expect("outer cell");
        let note_noun = outer.head();
        assert!(note_noun.is_cell(), "note must be a cell");

        let rest = outer.tail();
        assert!(rest.is_cell(), "rest [mani root] must be a cell");
    }

    #[test]
    fn loobean_encoding() {
        assert_eq!(make_loobean(true).as_atom().unwrap().as_u64().unwrap(), 0);
        assert_eq!(make_loobean(false).as_atom().unwrap().as_u64().unwrap(), 1);
    }

    #[test]
    fn cord_encoding() {
        let mut stack = new_stack();
        let abc = make_cord(&mut stack, "abc");
        let val = abc.as_atom().unwrap().as_u64().unwrap();
        assert_eq!(val, 97 + 98 * 256 + 99 * 65536);
    }

    #[test]
    fn tag_pending_encoding() {
        let mut stack = new_stack();
        let tag = make_atom(&mut stack, b"pending");
        let expected: u64 = b"pending"
            .iter()
            .enumerate()
            .map(|(i, &b)| (b as u64) << (i * 8))
            .sum();
        let val = tag.as_atom().unwrap().as_u64().unwrap();
        assert_eq!(val, expected);
    }

    #[test]
    fn list_encoding_structure() {
        let mut stack = new_stack();

        let proof = vec![
            ProofNode {
                hash: [0xAA; 5],
                side: true,
            },
            ProofNode {
                hash: [0xBB; 5],
                side: false,
            },
        ];
        let list = proof_list_to_noun(&mut stack, &proof);

        assert!(list.is_cell(), "list must be a cell");
        let first = list.as_cell().unwrap();
        assert!(
            first.head().is_cell(),
            "first element must be a cell [hash side]"
        );

        let rest = first.tail();
        assert!(rest.is_cell(), "rest must be a cell [node1 0]");
        let second = rest.as_cell().unwrap();
        assert!(second.head().is_cell(), "second element must be a cell");

        let term = second.tail();
        assert!(term.is_atom(), "terminator must be atom 0");
        assert_eq!(term.as_atom().unwrap().as_u64().unwrap(), 0);
    }
}
