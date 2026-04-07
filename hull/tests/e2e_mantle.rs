//! Integration test: vesl-mantle <-> hull type alignment.
//!
//! Verifies the Sigil -> Vigil -> settle poke pipeline works
//! end-to-end using vesl-mantle types re-exported through hull.

use vesl_mantle::{Sigil, Vigil, RagVerifier, Anchor};
use vesl_mantle::types::{Chunk, Retrieval, Manifest, Note, NoteState};
use vesl_mantle::anchor::build_settle_poke;

#[test]
fn sigil_vigil_settle_pipeline() {
    // 1. Build a Sigil tree from test chunks
    let chunks: Vec<&[u8]> = vec![
        b"Q3 revenue: $4.2M ARR",
        b"Risk exposure: $800K",
        b"Board approved Series B",
    ];
    let mut sigil = Sigil::new();
    let root = sigil.commit(&chunks);

    // 2. Verify with Vigil
    let mut vigil = Vigil::new();
    vigil.register_root(root);

    for (i, chunk) in chunks.iter().enumerate() {
        let proof = sigil.proof(i);
        assert!(vigil.check(chunk, &proof, &root), "chunk {i} proof failed");
    }

    // 3. Build manifest and verify via Vigil
    let retrievals: Vec<Retrieval> = chunks
        .iter()
        .enumerate()
        .map(|(i, c)| Retrieval {
            chunk: Chunk {
                id: i as u64,
                dat: String::from_utf8_lossy(c).into_owned(),
            },
            proof: sigil.proof(i),
            score: 950_000,
        })
        .collect();

    let mut prompt = String::from("Summarize financials");
    for r in &retrievals {
        prompt.push('\n');
        prompt.push_str(&r.chunk.dat);
    }

    let manifest = Manifest {
        query: "Summarize financials".into(),
        results: retrievals,
        prompt,
        output: "Financials look good.".into(),
        page: 0,
    };

    assert!(vigil.check_manifest(&manifest, &root));

    // 4. Build settle poke via free function
    let note = Note {
        id: 1,
        hull: 7,
        root,
        state: NoteState::Pending,
    };
    let slab = build_settle_poke(&note, &manifest, &root);
    // SAFETY: root was set in build_settle_poke via slab.set_root()
    assert!(unsafe { slab.root() }.is_cell(), "settle poke must be a cell");
}

#[test]
fn rag_verifier_through_graft_payload() {
    let chunks: Vec<&[u8]> = vec![b"alpha", b"bravo"];
    let mut sigil = Sigil::new();
    let root = sigil.commit(&chunks);

    let retrievals: Vec<Retrieval> = chunks
        .iter()
        .enumerate()
        .map(|(i, c)| Retrieval {
            chunk: Chunk {
                id: i as u64,
                dat: String::from_utf8_lossy(c).into_owned(),
            },
            proof: sigil.proof(i),
            score: 900_000,
        })
        .collect();

    let mut prompt = String::from("test query");
    for r in &retrievals {
        prompt.push('\n');
        prompt.push_str(&r.chunk.dat);
    }

    let manifest = Manifest {
        query: "test query".into(),
        results: retrievals,
        prompt,
        output: "test output".into(),
        page: 0,
    };

    let data = serde_json::to_vec(&manifest).unwrap();
    let verifier = RagVerifier;
    assert!(vesl_mantle::IntentVerifier::verify(&verifier, &data, &root));
}

#[tokio::test]
async fn anchor_settle_manifest_e2e() {
    let chunks: Vec<&[u8]> = vec![b"chunk-one", b"chunk-two"];
    let mut sigil = Sigil::new();
    let root = sigil.commit(&chunks);

    let retrievals: Vec<Retrieval> = chunks
        .iter()
        .enumerate()
        .map(|(i, c)| Retrieval {
            chunk: Chunk {
                id: i as u64,
                dat: String::from_utf8_lossy(c).into_owned(),
            },
            proof: sigil.proof(i),
            score: 800_000,
        })
        .collect();

    let mut prompt = String::from("query");
    for r in &retrievals {
        prompt.push('\n');
        prompt.push_str(&r.chunk.dat);
    }

    let manifest = Manifest {
        query: "query".into(),
        results: retrievals,
        prompt,
        output: "output".into(),
        page: 0,
    };

    let note = Note {
        id: 99,
        hull: 7,
        root,
        state: NoteState::Pending,
    };

    let mut anchor = Anchor::without_kernel();
    anchor.register_root(root);

    let settled = anchor.settle_manifest(&note, &manifest, &root).await.unwrap();
    assert!(matches!(settled.state, NoteState::Settled));
    assert_eq!(settled.id, 99);
}
