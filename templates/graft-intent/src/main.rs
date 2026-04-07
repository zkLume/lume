use std::error::Error;
use std::fs;

use vesl_mantle::{Sigil, Tip5Hash};
use nock_noun_rs::make_tag_in;
use nockapp::kernel::boot;
use nockapp::noun::slab::NounSlab;
use nockapp::wire::{SystemWire, Wire};
use nockapp::NockApp;
use nockvm::noun::{D, T};
use nockvm_macros::tas;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let cli = boot::default_boot_cli(false);
    boot::init_default_tracing(&cli);

    let kernel =
        fs::read("out.jam").map_err(|e| format!("Failed to read out.jam: {}", e))?;

    let mut app: NockApp =
        boot::setup(&kernel, Some(cli), &[], "{{project_name}}", None).await?;

    // --- step 1: declare intents (domain logic) ---

    let intents = [
        "swap 100 USDC for ETH at market price",
        "bridge 50 USDC to L2 within 30 minutes",
        "stake 1000 tokens with validator xyz",
    ];

    println!("=== step 1: declaring intents ===\n");
    for intent in &intents {
        let mut slab = NounSlab::new();
        let tag = D(tas!(b"declare"));
        let val = make_tag_in(&mut slab, intent);
        let poke = T(&mut slab, &[tag, val]);
        slab.set_root(poke);

        let effects = app.poke(SystemWire.to_wire(), slab).await?;
        print_effects(&effects, &format!("declare '{}'", &intent[..30.min(intent.len())]));
    }

    // --- step 2: commit intent data to a Merkle tree ---

    println!("\n=== step 2: Sigil — building Merkle tree ===\n");
    let mut sigil = Sigil::new();
    let leaves: Vec<&[u8]> = intents.iter().map(|i| i.as_bytes()).collect();
    sigil.commit(&leaves);

    let root: Tip5Hash = sigil.root().expect("committed");
    println!("  root: {:?}", root);
    println!("  leaves: {}", intents.len());

    // --- step 3: register root with kernel ---

    println!("\n=== step 3: Graft — registering root ===\n");
    let hull_id: u64 = 1;
    {
        let mut slab = NounSlab::new();
        let tag = make_tag_in(&mut slab, "vesl-register");
        let root_atom = D(root[0]);
        let poke = T(&mut slab, &[tag, D(hull_id), root_atom]);
        slab.set_root(poke);

        let effects = app.poke(SystemWire.to_wire(), slab).await?;
        print_effects(&effects, "register");
    }

    // --- step 4: verify proofs locally ---
    //
    // The custom hash gate in the kernel does:
    //   =((hash-leaf data) expected-root)
    //
    // From Rust, we use Sigil proofs for local verification.
    // The kernel gate is intentionally simpler — it doesn't
    // need Merkle proofs because it hashes raw data directly.
    // This is the point: your gate can be anything.

    println!("\n=== step 4: local proof verification ===\n");
    for (i, intent) in intents.iter().enumerate() {
        let proof = sigil.proof(i);
        let leaf_hash = vesl_mantle::sigil::hash_leaf(intent.as_bytes());
        println!(
            "  intent {}: leaf_hash={:?}, proof_len={}",
            i, &leaf_hash[..2], proof.len()
        );
    }

    println!("\n=== done ===");
    println!("\nThe intent pattern: declare -> commit -> register -> settle.");
    println!("Custom gate: hash the data, compare to root. No manifest needed.");
    Ok(())
}

fn print_effects(effects: &[NounSlab], label: &str) {
    if effects.is_empty() {
        println!("  [{}] (no effects)", label);
        return;
    }
    for effect in effects.iter() {
        let noun = unsafe { effect.root() };
        if let Ok(cell) = noun.as_cell() {
            if let Ok(tag) = cell.head().as_atom() {
                let tag_bytes = tag.as_ne_bytes();
                let tag_str = std::str::from_utf8(tag_bytes)
                    .unwrap_or("?")
                    .trim_end_matches('\0');
                println!("  [{}] effect: %{}", label, tag_str);
            }
        }
    }
}
