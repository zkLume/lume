use std::error::Error;
use std::fs;

use vesl_core::{Guard, Mint, Tip5Hash, tip5_to_atom_le_bytes};
use nock_noun_rs::{jam_to_bytes, make_atom_in, make_cord_in, make_tag_in, new_stack};
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
        boot::setup(&kernel, cli, &[], "{{project_name}}", None).await?;

    // --- step 1: submit reports (domain logic) ---

    let reports = [
        ("Q3 Revenue", "Revenue up 12% YoY. APAC expansion on track."),
        ("Risk Assessment", "Supply chain exposure reduced. New vendor onboarded."),
        ("Compliance Audit", "SOC2 Type II passed. Zero critical findings."),
    ];

    println!("=== step 1: submitting reports ===\n");
    for (title, body) in &reports {
        let mut slab = NounSlab::new();
        let tag = D(tas!(b"submit"));
        let t = make_cord_in(&mut slab, title);
        let b = make_cord_in(&mut slab, body);
        let poke = T(&mut slab, &[tag, t, b]);
        slab.set_root(poke);

        let effects = app.poke(SystemWire.to_wire(), slab).await?;
        print_effects(&effects, &format!("submit '{}'", title));
    }

    // --- step 2: commit report bodies to Merkle tree ---

    println!("\n=== step 2: Mint — building Merkle tree ===\n");
    let mut mint = Mint::new();
    let leaves: Vec<&[u8]> = reports.iter().map(|(_, body)| body.as_bytes()).collect();
    mint.commit(&leaves);

    let root: Tip5Hash = mint.root().expect("committed");
    println!("  root: {:?}", root);
    println!("  leaves: {}", reports.len());

    // --- step 3: register root with kernel ---

    println!("\n=== step 3: Graft — registering root ===\n");
    let hull_id: u64 = 1;
    {
        let mut slab = NounSlab::new();
        let tag = make_tag_in(&mut slab, "settle-register");
        let root_bytes = tip5_to_atom_le_bytes(&root);
        let root_atom = make_atom_in(&mut slab, &root_bytes);
        let poke = T(&mut slab, &[tag, D(hull_id), root_atom]);
        slab.set_root(poke);

        let effects = app.poke(SystemWire.to_wire(), slab).await?;
        print_effects(&effects, "register");
    }

    // --- step 4: verify all proofs locally with Guard ---

    println!("\n=== step 4: Guard — local verification ===\n");
    let mut guard = Guard::new();
    guard.register_root(root).unwrap();

    for (i, (title, body)) in reports.iter().enumerate() {
        let proof = mint.proof(i).unwrap();
        let valid = guard.check(body.as_bytes(), &proof, &root);
        println!("  {} '{}': {}", if valid { "ok" } else { "FAIL" }, title, valid);
    }

    // --- step 5: settle a report ---
    //
    // Build a graft-payload noun, jam it, and poke %settle-note.
    // The kernel's Graft verifies via the hash gate, then transitions
    // the note to %settled. Replay protection prevents double-settlement.
    //
    // For the hash gate to pass, we need a single-leaf tree where
    // root == hash-leaf(data). Multi-leaf roots need a manifest gate.

    println!("\n=== step 5: settlement ===\n");

    let mut single_mint = Mint::new();
    let single_root = single_mint.commit(&[reports[0].1.as_bytes()]);

    // Register the single-leaf root under a separate hull
    let settle_hull: u64 = 2;
    {
        let mut slab = NounSlab::new();
        let tag = make_tag_in(&mut slab, "settle-register");
        let rb = tip5_to_atom_le_bytes(&single_root);
        let root_atom = make_atom_in(&mut slab, &rb);
        let poke = T(&mut slab, &[tag, D(settle_hull), root_atom]);
        slab.set_root(poke);
        app.poke(SystemWire.to_wire(), slab).await?;
    }

    // Build and send %settle-note
    //
    // graft-payload: [note=[id=@ hull=@ root=@ state=[%pending ~]] data=* expected-root=@]
    {
        let mut slab = NounSlab::new();
        let rb = tip5_to_atom_le_bytes(&single_root);

        let note_id = D(1);
        let note_hull = D(settle_hull);
        let note_root = make_atom_in(&mut slab, &rb);
        let pending_tag = make_tag_in(&mut slab, "pending");
        let state = T(&mut slab, &[pending_tag, D(0)]);
        let note = T(&mut slab, &[note_id, note_hull, note_root, state]);

        let data = make_atom_in(&mut slab, reports[0].1.as_bytes());
        let exp_root = make_atom_in(&mut slab, &rb);
        let payload_noun = T(&mut slab, &[note, data, exp_root]);

        let payload_bytes = {
            let mut stack = new_stack();
            jam_to_bytes(&mut stack, payload_noun)
        };
        let jammed = make_atom_in(&mut slab, &payload_bytes);
        let tag = make_tag_in(&mut slab, "settle-note");
        let poke = T(&mut slab, &[tag, jammed]);
        slab.set_root(poke);

        let effects = app.poke(SystemWire.to_wire(), slab).await?;
        print_effects(&effects, "settle-note");
    }

    // Replay protection: same note ID should produce %settle-error
    {
        let mut slab = NounSlab::new();
        let rb = tip5_to_atom_le_bytes(&single_root);

        let note = T(&mut slab, &[
            D(1), D(settle_hull),
            make_atom_in(&mut slab, &rb),
            T(&mut slab, &[make_tag_in(&mut slab, "pending"), D(0)]),
        ]);
        let data = make_atom_in(&mut slab, reports[0].1.as_bytes());
        let exp_root = make_atom_in(&mut slab, &rb);
        let payload_noun = T(&mut slab, &[note, data, exp_root]);

        let payload_bytes = {
            let mut stack = new_stack();
            jam_to_bytes(&mut stack, payload_noun)
        };
        let jammed = make_atom_in(&mut slab, &payload_bytes);
        let tag = make_tag_in(&mut slab, "settle-note");
        let poke = T(&mut slab, &[tag, jammed]);
        slab.set_root(poke);

        let effects = app.poke(SystemWire.to_wire(), slab).await?;
        print_effects(&effects, "replay (expect error)");
    }

    // --- step 6: tampered data detection ---

    println!("\n=== step 6: tampered data detection ===\n");
    let proof = mint.proof(0);
    let tampered = guard.check(b"Revenue down 50%. CEO arrested.", &proof, &root);
    println!("  tampered report: valid={}", tampered);

    let bad_root: Tip5Hash = [0xDEAD, 0, 0, 0, 0];
    let unreg = guard.check(reports[0].1.as_bytes(), &proof, &bad_root);
    println!("  unregistered root: valid={}", unreg);

    println!("\n=== done ===");
    println!("\nThe Settle pattern: submit -> commit -> register -> settle.");
    println!("Replay rejected. Your domain logic stays clean.");
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
