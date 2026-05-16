//! Nock noun construction — RAG-specific builders.
//!
//! Re-exports generic builders from vesl-core and adds RAG-specific
//! structures (manifest, settlement payload).

pub use vesl_core::noun_builder::{
    hash_to_noun, hash_to_noun_generic,
    proof_node_to_noun, proof_list_to_noun,
    chunk_to_noun, retrieval_to_noun, retrieval_list_to_noun,
    pending_note_to_noun, build_register_poke,
};
pub use vesl_core::settle::{build_settle_poke, build_prove_poke};

use nock_noun_rs::{
    make_cord, NockStack, Noun, T,
};

#[cfg(test)]
use crate::merkle::MerkleTree;
use crate::types::*;

// ---------------------------------------------------------------------------
// RAG-specific builders
// ---------------------------------------------------------------------------

/// `+$manifest  [query=@t results=(list retrieval) prompt=@t output=@t page=@ud]`
fn manifest_to_noun(stack: &mut NockStack, m: &Manifest) -> Noun {
    let query = make_cord(stack, &m.query);
    let results = retrieval_list_to_noun(stack, &m.results);
    let prompt = make_cord(stack, &m.prompt);
    let output = make_cord(stack, &m.output);
    let page = nock_noun_rs::D(m.page);
    T(stack, &[query, results, prompt, output, page])
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
    nock_noun_rs::jam_to_bytes(payload, &stack.noun_space())
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
        page: 0,
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
    use nock_noun_rs::{cue, jam, new_stack};

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

        // V-L07: use tempfile so test artifacts are cleaned up automatically
        let mut tmp = tempfile::NamedTempFile::new().expect("create temp file");
        std::io::Write::write_all(&mut tmp, &jam_bytes).expect("write jam file");
        let jam_path = tmp.path();
        println!("Wrote {} bytes to {}", jam_bytes.len(), jam_path.display());

        let read_back = std::fs::read(jam_path).expect("read jam file");
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

        let space = stack.noun_space();
        let outer = nockvm::noun::NounHandle::new(cued, &space)
            .as_cell()
            .expect("outer cell");
        let note_noun = outer.head();
        assert!(note_noun.is_cell(), "note must be a cell");

        let rest = outer.tail();
        assert!(rest.is_cell(), "rest [mani root] must be a cell");
    }
}
