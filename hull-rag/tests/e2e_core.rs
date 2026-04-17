//! Integration test: vesl-core <-> hull type alignment.
//!
//! Verifies the Mint -> Guard -> settle poke pipeline works
//! end-to-end using vesl-core types re-exported through hull.

use vesl_core::{Mint, Guard, RagVerifier, Settle};
use vesl_core::types::{Chunk, Retrieval, Manifest, Note, NoteState};
use vesl_core::settle::build_settle_poke;

#[test]
fn mint_guard_settle_pipeline() {
    // 1. Build a Mint tree from test chunks
    let chunks: Vec<&[u8]> = vec![
        b"Q3 revenue: $4.2M ARR",
        b"Risk exposure: $800K",
        b"Board approved Series B",
    ];
    let mut mint = Mint::new();
    let root = mint.commit(&chunks);

    // 2. Verify with Guard
    let mut guard = Guard::new();
    guard.register_root(root);

    for (i, chunk) in chunks.iter().enumerate() {
        let proof = mint.proof(i).unwrap();
        assert!(guard.check(chunk, &proof, &root), "chunk {i} proof failed");
    }

    // 3. Build manifest and verify via Guard
    let retrievals: Vec<Retrieval> = chunks
        .iter()
        .enumerate()
        .map(|(i, c)| Retrieval {
            chunk: Chunk {
                id: i as u64,
                dat: String::from_utf8_lossy(c).into_owned(),
            },
            proof: mint.proof(i).unwrap(),
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

    assert!(guard.check_manifest(&manifest, &root));

    // 4. Build settle poke via free function
    let note = Note {
        id: 1,
        hull: 7,
        root,
        state: NoteState::Pending,
    };
    let slab = build_settle_poke(&note, &manifest, &root);
    let root_noun = nock_noun_rs::slab_root(&slab);
    assert!(root_noun.is_cell(), "settle poke must be a cell");
}

#[test]
fn rag_verifier_through_graft_payload() {
    let chunks: Vec<&[u8]> = vec![b"alpha", b"bravo"];
    let mut mint = Mint::new();
    let root = mint.commit(&chunks);

    let retrievals: Vec<Retrieval> = chunks
        .iter()
        .enumerate()
        .map(|(i, c)| Retrieval {
            chunk: Chunk {
                id: i as u64,
                dat: String::from_utf8_lossy(c).into_owned(),
            },
            proof: mint.proof(i).unwrap(),
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
    assert!(vesl_core::IntentVerifier::verify(&verifier, 1, &data, &root));
}

#[tokio::test]
async fn settle_manifest_e2e() {
    let chunks: Vec<&[u8]> = vec![b"chunk-one", b"chunk-two"];
    let mut mint = Mint::new();
    let root = mint.commit(&chunks);

    let retrievals: Vec<Retrieval> = chunks
        .iter()
        .enumerate()
        .map(|(i, c)| Retrieval {
            chunk: Chunk {
                id: i as u64,
                dat: String::from_utf8_lossy(c).into_owned(),
            },
            proof: mint.proof(i).unwrap(),
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

    let mut settler = Settle::without_kernel();
    settler.register_root(root).unwrap();

    let settled = settler.settle_manifest(&note, &manifest, &root).await.unwrap();
    assert!(matches!(settled.state, NoteState::Settled));
    assert_eq!(settled.id, 99);
}

/// Full kernel integration: boot the vesl kernel, register a root,
/// dispatch a settle poke, and verify the kernel accepts it.
///
/// This exercises the path that `Settle::poke_bytes()` enables:
/// SDK builds the poke, hull dispatches it to the real kernel.
#[tokio::test]
async fn settle_poke_through_kernel() {
    use clap::Parser;
    use nockapp::kernel::boot;
    use nockapp::NockApp;
    use nockapp::wire::{SystemWire, Wire};

    // Boot a throwaway kernel
    let cli = boot::Cli::parse_from(["test", "--new"]);
    let mut app: NockApp = boot::setup(kernels_vesl::KERNEL, cli, &[], "vesl-test", None)
        .await
        .expect("kernel boot failed");

    // Build test data
    let chunks: Vec<&[u8]> = vec![b"alpha-data", b"bravo-data"];
    let mut mint = Mint::new();
    let root = mint.commit(&chunks);

    // Register root with kernel
    let register_poke = vesl_core::settle::build_register_poke(7, &root);
    let reg_effects = app.poke(SystemWire.to_wire(), register_poke).await
        .expect("register poke failed");
    assert!(!reg_effects.is_empty(), "register poke should return effects");

    // Build manifest
    let retrievals: Vec<Retrieval> = chunks
        .iter()
        .enumerate()
        .map(|(i, c)| Retrieval {
            chunk: Chunk {
                id: i as u64,
                dat: String::from_utf8_lossy(c).into_owned(),
            },
            proof: mint.proof(i).unwrap(),
            score: 800_000,
        })
        .collect();

    let mut prompt = String::from("test-query");
    for r in &retrievals {
        prompt.push('\n');
        prompt.push_str(&r.chunk.dat);
    }

    let manifest = Manifest {
        query: "test-query".into(),
        results: retrievals,
        prompt,
        output: "test-output".into(),
        page: 0,
    };

    let note = Note {
        id: 1,
        hull: 7,
        root,
        state: NoteState::Pending,
    };

    // Build the settle poke via SDK
    let settle_poke = build_settle_poke(&note, &manifest, &root);

    // Dispatch to kernel — this is the gap we're closing
    let effects = app.poke(SystemWire.to_wire(), settle_poke).await
        .expect("settle poke failed — kernel crashed on valid input");
    assert!(!effects.is_empty(), "settle poke should return effects (note settled)");
}
