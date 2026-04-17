//! Forge kernel integration tests — CORE_KERNELS.md verification step 7.
//!
//! Tests the forge kernel's four poke handlers:
//!   %register → %settle → %verify → %prove
//!
//! The %prove test requires 128GB RAM + STARK prover jets and is
//! marked #[ignore] — run explicitly on the PC:
//!
//!   cargo test --test e2e_forge -- --nocapture --ignored
//!
//! Non-prove tests run on any machine.

use clap::Parser;
use nockapp::kernel::boot;
use nockapp::wire::{SystemWire, Wire};
use nockapp::NockApp;
use tempfile::TempDir;

use vesl_core::forge::{
    build_forge_prove_poke, build_forge_settle_poke, build_forge_verify_poke,
    extract_proof_from_effects,
};
use vesl_core::settle::build_register_poke;
use vesl_core::types::{ForgePayload, LeafWithProof, Note, NoteState};
use vesl_core::{MerkleTree, Tip5Hash};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Boot the forge kernel with an isolated temp directory.
/// No prover jets — use `boot_forge_with_prover` for %prove tests.
async fn boot_forge() -> (NockApp, TempDir) {
    let tmp = TempDir::new().expect("create temp dir");
    let cli = boot::Cli::parse_from(["test", "--new"]);
    let app = boot::setup(
        kernels_forge::KERNEL,
        cli,
        &[],
        "forge",
        Some(tmp.path().to_path_buf()),
    )
    .await
    .expect("forge kernel must boot");
    (app, tmp)
}

/// Boot the forge kernel with STARK prover jets (for %prove).
async fn boot_forge_with_prover() -> (NockApp, TempDir) {
    let tmp = TempDir::new().expect("create temp dir");
    let cli = boot::Cli::parse_from(["test", "--new", "--stack-size", "large"]);
    let prover_hot_state = zkvm_jetpack::hot::produce_prover_hot_state();
    let app = boot::setup(
        kernels_forge::KERNEL,
        cli,
        prover_hot_state.as_slice(),
        "forge",
        Some(tmp.path().to_path_buf()),
    )
    .await
    .expect("forge kernel must boot with prover jets");
    (app, tmp)
}

/// Build a test tree + forge payload from raw byte chunks.
fn build_test_payload(hull_id: u64, note_id: u64, chunks: &[&[u8]]) -> (ForgePayload, Tip5Hash) {
    let tree = MerkleTree::build(chunks);
    let root = tree.root();

    let leaves: Vec<LeafWithProof> = chunks
        .iter()
        .enumerate()
        .map(|(i, c)| LeafWithProof {
            dat: c.to_vec(),
            proof: tree.proof(i),
        })
        .collect();

    let payload = ForgePayload {
        note: Note {
            id: note_id,
            hull: hull_id,
            root,
            state: NoteState::Pending,
        },
        leaves,
        expected_root: root,
    };

    (payload, root)
}

// ---------------------------------------------------------------------------
// Tests: kernel boot
// ---------------------------------------------------------------------------

#[tokio::test]
async fn forge_kernel_boots() {
    let (_app, _tmp) = boot_forge().await;
    println!("  forge kernel booted ({} bytes JAM)", kernels_forge::KERNEL.len());
}

// ---------------------------------------------------------------------------
// Tests: %register
// ---------------------------------------------------------------------------

#[tokio::test]
async fn forge_register_produces_effect() {
    let (mut app, _tmp) = boot_forge().await;
    let root: Tip5Hash = [1, 2, 3, 4, 5];

    let poke = build_register_poke(7, &root);
    let effects = app.poke(SystemWire.to_wire(), poke).await.expect("register");
    assert!(!effects.is_empty(), "register must produce [%registered hull root]");
}

#[tokio::test]
async fn forge_register_duplicate_rejected() {
    let (mut app, _tmp) = boot_forge().await;
    let root: Tip5Hash = [1, 2, 3, 4, 5];

    let poke1 = build_register_poke(7, &root);
    app.poke(SystemWire.to_wire(), poke1).await.expect("first register");

    let poke2 = build_register_poke(7, &root);
    let effects = app.poke(SystemWire.to_wire(), poke2).await.expect("duplicate register");
    assert!(effects.is_empty(), "duplicate register must produce 0 effects");
}

// ---------------------------------------------------------------------------
// Tests: %settle
// ---------------------------------------------------------------------------

#[tokio::test]
async fn forge_settle_single_leaf() {
    let (mut app, _tmp) = boot_forge().await;
    let chunks: Vec<&[u8]> = vec![b"single-leaf-test"];
    let (payload, root) = build_test_payload(7, 1, &chunks);

    // Register first
    let reg = build_register_poke(7, &root);
    app.poke(SystemWire.to_wire(), reg).await.expect("register");

    // Settle
    let settle = build_forge_settle_poke(&payload);
    let effects = app.poke(SystemWire.to_wire(), settle).await.expect("settle poke");
    assert!(!effects.is_empty(), "settle must produce [id hull root [%settled ~]]");
    println!("  settle single-leaf: {} effects", effects.len());
}

#[tokio::test]
async fn forge_settle_multi_leaf() {
    let (mut app, _tmp) = boot_forge().await;
    let chunks: Vec<&[u8]> = vec![
        b"Q3 revenue: $4.2M ARR",
        b"Risk exposure: $800K",
        b"Board approved Series B",
        b"SOC2 audit scheduled",
    ];
    let (payload, root) = build_test_payload(42, 1, &chunks);

    let reg = build_register_poke(42, &root);
    app.poke(SystemWire.to_wire(), reg).await.expect("register");

    let settle = build_forge_settle_poke(&payload);
    let effects = app.poke(SystemWire.to_wire(), settle).await.expect("settle poke");
    assert!(!effects.is_empty(), "multi-leaf settle must produce effects");
    println!("  settle 4-leaf: {} effects", effects.len());
}

#[tokio::test]
async fn forge_settle_replay_rejected() {
    let (mut app, _tmp) = boot_forge().await;
    let chunks: Vec<&[u8]> = vec![b"replay-test"];
    let (payload, root) = build_test_payload(7, 1, &chunks);

    let reg = build_register_poke(7, &root);
    app.poke(SystemWire.to_wire(), reg).await.expect("register");

    // First settle
    let settle1 = build_forge_settle_poke(&payload);
    let effects1 = app.poke(SystemWire.to_wire(), settle1).await.expect("first settle");
    assert!(!effects1.is_empty(), "first settle must succeed");

    // Replay with same note ID
    let settle2 = build_forge_settle_poke(&payload);
    let effects2 = app.poke(SystemWire.to_wire(), settle2).await.expect("replay settle");
    assert!(effects2.is_empty(), "replay must produce 0 effects (note already settled)");
}

#[tokio::test]
async fn forge_settle_unregistered_root_rejected() {
    let (mut app, _tmp) = boot_forge().await;
    let chunks: Vec<&[u8]> = vec![b"unregistered-test"];
    let (payload, _root) = build_test_payload(7, 1, &chunks);

    // Skip registration — settle should fail
    let settle = build_forge_settle_poke(&payload);
    let effects = app.poke(SystemWire.to_wire(), settle).await.expect("settle poke");
    assert!(effects.is_empty(), "unregistered root must produce 0 effects");
}

// ---------------------------------------------------------------------------
// Tests: %verify
// ---------------------------------------------------------------------------

#[tokio::test]
async fn forge_verify_valid_payload() {
    let (mut app, _tmp) = boot_forge().await;
    let chunks: Vec<&[u8]> = vec![b"verify-chunk-a", b"verify-chunk-b"];
    let (payload, root) = build_test_payload(7, 1, &chunks);

    let reg = build_register_poke(7, &root);
    app.poke(SystemWire.to_wire(), reg).await.expect("register");

    let verify = build_forge_verify_poke(&payload);
    let effects = app.poke(SystemWire.to_wire(), verify).await.expect("verify poke");
    assert!(!effects.is_empty(), "valid verify must produce [%verified ok=?]");

    // Verify is read-only — same payload should work again (no state change)
    let verify2 = build_forge_verify_poke(&payload);
    let effects2 = app.poke(SystemWire.to_wire(), verify2).await.expect("re-verify");
    assert!(!effects2.is_empty(), "verify is idempotent (read-only)");
}

#[tokio::test]
async fn forge_verify_bad_root_returns_false() {
    let (mut app, _tmp) = boot_forge().await;
    let chunks: Vec<&[u8]> = vec![b"bad-root-test"];
    let (payload, _root) = build_test_payload(7, 1, &chunks);

    // Register a different root than what the payload expects
    let different_root: Tip5Hash = [99, 99, 99, 99, 99];
    let reg = build_register_poke(7, &different_root);
    app.poke(SystemWire.to_wire(), reg).await.expect("register");

    // Verify should return [%verified %.n] — root mismatch
    let verify = build_forge_verify_poke(&payload);
    let effects = app.poke(SystemWire.to_wire(), verify).await.expect("verify poke");
    assert!(!effects.is_empty(), "verify must return [%verified ?] even on failure");

    // Check the effect is [%verified %.n] (loobean false = atom 1)
    let slab = &effects[0];
    let root_noun = nock_noun_rs::slab_root(slab);
    let cell = root_noun.as_cell().expect("effect must be a cell");
    let tag = cell.head().as_atom().expect("tag is an atom");
    let tag_bytes = tag.as_ne_bytes();
    let len = tag_bytes.iter().rposition(|&b| b != 0).map_or(0, |p| p + 1);
    assert_eq!(&tag_bytes[..len], b"verified", "effect tag must be %verified");
    let ok_val = cell.tail().as_atom().expect("ok=? is an atom").as_u64().unwrap();
    assert_eq!(ok_val, 1, "root mismatch must return %.n (loobean false = 1)");
}

// ---------------------------------------------------------------------------
// Tests: %prove (STARK prover — PC only, 128GB)
// ---------------------------------------------------------------------------

/// Full prove roundtrip: register → %prove → extract proof.
///
/// This test boots the kernel with STARK prover jets and a large NockStack.
/// Run on the PC only:
///   cargo test --test e2e_forge forge_prove_roundtrip -- --nocapture --ignored
#[tokio::test]
#[ignore]
async fn forge_prove_roundtrip() {
    println!("[forge] Booting kernel with prover jets...");
    let (mut app, _tmp) = boot_forge_with_prover().await;
    println!("[forge] Kernel booted ({} bytes JAM)", kernels_forge::KERNEL.len());

    let chunks: Vec<&[u8]> = vec![
        b"Verified computation on Nockchain",
        b"Forge tier four of the Vesl SDK",
    ];
    let (payload, root) = build_test_payload(7, 1, &chunks);

    // Register root
    println!("[forge] Registering root...");
    let reg = build_register_poke(7, &root);
    let reg_effects = app.poke(SystemWire.to_wire(), reg).await.expect("register");
    assert!(!reg_effects.is_empty(), "register must produce effects");
    println!("[forge] Root registered.");

    // Fire %prove
    println!("[forge] Sending %prove poke (this may take minutes)...");
    let prove = build_forge_prove_poke(&payload);
    let prove_result = tokio::time::timeout(
        std::time::Duration::from_secs(600),
        app.poke(SystemWire.to_wire(), prove),
    )
    .await;

    let effects = match prove_result {
        Ok(Ok(effs)) => effs,
        Ok(Err(e)) => panic!("[forge] %prove poke failed: {e}"),
        Err(_) => panic!("[forge] %prove timed out after 600s"),
    };

    println!("[forge] %prove returned {} effects", effects.len());
    assert!(!effects.is_empty(), "prove must produce effects");

    // Extract proof (JAMs the proof noun — handles both atom and cell proofs)
    let proof = extract_proof_from_effects(&effects)
        .expect("effect parsing must not error");
    match proof {
        Some(bytes) => {
            println!("[forge] STARK proof extracted: {} bytes (JAM'd)", bytes.len());
            assert!(bytes.len() > 100, "proof must be non-trivial (got {} bytes)", bytes.len());
        }
        None => {
            panic!("[forge] No proof extracted from effects");
        }
    }
}
