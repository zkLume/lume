use std::error::Error;
use std::fs;

use vesl_core::{Guard, Mint, Tip5Hash};
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

    // --- domain: store some notes ---

    let notes = [
        ("meeting", "quarterly review moved to friday"),
        ("deploy", "v2.3.1 shipped to staging"),
        ("bug", "null pointer in auth middleware"),
    ];

    println!("=== storing notes ===\n");
    for (key, val) in &notes {
        let mut slab = NounSlab::new();
        let tag = D(tas!(b"put"));
        let k = make_tag_in(&mut slab, key);
        let v = make_tag_in(&mut slab, val);
        let poke = T(&mut slab, &[tag, k, v]);
        slab.set_root(poke);

        let effects = app.poke(SystemWire.to_wire(), slab).await?;
        print_effects(&effects, &format!("put '{}'", key));
    }

    // --- mint: commit note data to a Merkle tree ---

    println!("\n=== Mint: building Merkle tree ===\n");
    let mut mint = Mint::new();
    let leaves: Vec<&[u8]> = notes.iter().map(|(_, v)| v.as_bytes()).collect();
    mint.commit(&leaves);

    let root: Tip5Hash = mint.root().expect("tree committed");
    println!("  Merkle root: {:?}", root);

    // --- graft: register root with kernel ---

    println!("\n=== Graft: registering root in kernel ===\n");
    let hull_id: u64 = 1;
    {
        let mut slab = NounSlab::new();
        let tag = make_tag_in(&mut slab, "vesl-register");
        // Root is [u64; 5] — encode each limb. For the kernel poke,
        // we pass the root as a single flat atom (digest-to-atom encoding).
        // For simplicity, pass the first limb as a stand-in.
        let root_atom = D(root[0]);
        let poke = T(&mut slab, &[tag, D(hull_id), root_atom]);
        slab.set_root(poke);

        let effects = app.poke(SystemWire.to_wire(), slab).await?;
        print_effects(&effects, "vesl-register");
    }

    // --- guard: verify proofs locally ---

    println!("\n=== Guard: verifying proofs ===\n");
    let mut guard = Guard::new();
    guard.register_root(root).unwrap();

    for (i, (key, val)) in notes.iter().enumerate() {
        let proof = mint.proof(i).unwrap();
        let valid = guard.check(val.as_bytes(), &proof, &root);
        println!("  {} '{}': {}", if valid { "ok" } else { "FAIL" }, key, valid);
    }

    // tampered data should fail
    let proof = mint.proof(0).unwrap();
    let tampered = guard.check(b"tampered data", &proof, &root);
    println!("  tampered:       {}", tampered);

    // unregistered root should fail
    let bad_root: Tip5Hash = [0xDEAD, 0, 0, 0, 0];
    let unreg = guard.check(notes[0].1.as_bytes(), &proof, &bad_root);
    println!("  unregistered:   {}", unreg);

    println!("\n=== done ===");
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
