use std::error::Error;
use std::fs;

use vesl_core::{Guard, Mint, Tip5Hash, tip5_to_atom_le_bytes};
use nock_noun_rs::{atom_from_u64, jam_to_bytes, make_atom_in, make_tag_in, new_stack};
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
        fs::read("out.jam").map_err(|e| format!("Failed to read out.jam: {e}"))?;

    let mut app: NockApp =
        boot::setup(&kernel, cli, &[], "my-nockapp", None).await?;

    // --- step 1: domain poke ---

    let items = [
        "first item added by the scaffold",
        "second item for demonstration",
    ];

    println!("=== step 1: domain pokes ===\n");
    for item in &items {
        let mut slab = NounSlab::new();
        let tag = D(tas!(b"my-action"));
        let val = make_tag_in(&mut slab, item);
        let poke = T(&mut slab, &[tag, val]);
        slab.set_root(poke);

        let effects = app.poke(SystemWire.to_wire(), slab).await?;
        print_effects(&effects, &format!("my-action '{}'", &item[..30.min(item.len())]));
    }

    // --- step 2: Mint — build Merkle tree ---

    println!("\n=== step 2: Mint — building Merkle tree ===\n");
    let mut mint = Mint::new();
    let leaves: Vec<&[u8]> = items.iter().map(|i| i.as_bytes()).collect();
    let root: Tip5Hash = mint.commit(&leaves);
    println!("  root: {:?}", root);
    println!("  leaves: {}", items.len());

    // --- step 3: Guard — verify proofs locally ---

    println!("\n=== step 3: Guard — local verification ===\n");
    let mut guard = Guard::new();
    guard.register_root(root).unwrap();

    for (i, item) in items.iter().enumerate() {
        let proof = mint.proof(i).unwrap();
        let valid = guard.check(item.as_bytes(), &proof, &root);
        println!("  item {i}: valid={valid}");
    }

    // --- step 4: register root with kernel ---

    println!("\n=== step 4: Graft — registering root ===\n");
    let hull_id: u64 = 1;
    {
        let mut slab = NounSlab::new();
        let tag = make_tag_in(&mut slab, "settle-register");
        let root_bytes = tip5_to_atom_le_bytes(&root);
        let root_atom = make_atom_in(&mut slab, &root_bytes);
        // atom_from_u64 handles values above DIRECT_MAX (2^63 − 1); prefer it
        // over D() whenever the value could be hash-derived.
        let hull = atom_from_u64(&mut slab, hull_id);
        let poke = T(&mut slab, &[tag, hull, root_atom]);
        slab.set_root(poke);

        let effects = app.poke(SystemWire.to_wire(), slab).await?;
        print_effects(&effects, "settle-register");
    }

    // --- step 5: settle ---
    //
    // Build a graft-payload, jam it, send as %settle-note.
    // The hash gate needs a single-leaf tree (root == hash-leaf(data)).

    println!("\n=== step 5: settlement ===\n");
    let mut single_mint = Mint::new();
    let single_root = single_mint.commit(&[items[0].as_bytes()]);

    // Register the single-leaf root under a second hull
    let settle_hull: u64 = 2;
    {
        let mut slab = NounSlab::new();
        let tag = make_tag_in(&mut slab, "settle-register");
        let rb = tip5_to_atom_le_bytes(&single_root);
        let root_atom = make_atom_in(&mut slab, &rb);
        let hull = atom_from_u64(&mut slab, settle_hull);
        let poke = T(&mut slab, &[tag, hull, root_atom]);
        slab.set_root(poke);
        app.poke(SystemWire.to_wire(), slab).await?;
    }

    // Build graft-payload: [note=[id hull root [%pending ~]] data expected-root]
    {
        let mut slab = NounSlab::new();
        let rb = tip5_to_atom_le_bytes(&single_root);

        let note_id = atom_from_u64(&mut slab, 1);
        let note_hull = atom_from_u64(&mut slab, settle_hull);
        let note_root = make_atom_in(&mut slab, &rb);
        let pending_tag = make_tag_in(&mut slab, "pending");
        let state = T(&mut slab, &[pending_tag, D(0)]);
        let note = T(&mut slab, &[note_id, note_hull, note_root, state]);

        let data = make_atom_in(&mut slab, items[0].as_bytes());
        let exp_root = make_atom_in(&mut slab, &rb);
        let payload_noun = T(&mut slab, &[note, data, exp_root]);

        // Jam the payload and send as [%settle-note jammed]
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

    // --- step 6: replay protection ---

    println!("\n=== step 6: replay protection ===\n");
    {
        let mut slab = NounSlab::new();
        let rb = tip5_to_atom_le_bytes(&single_root);

        let note_id = atom_from_u64(&mut slab, 1);
        let note_hull = atom_from_u64(&mut slab, settle_hull);
        let note = T(&mut slab, &[
            note_id, note_hull,
            make_atom_in(&mut slab, &rb),
            T(&mut slab, &[make_tag_in(&mut slab, "pending"), D(0)]),
        ]);
        let data = make_atom_in(&mut slab, items[0].as_bytes());
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

    println!("\n=== done ===");
    Ok(())
}

fn print_effects(effects: &[NounSlab], label: &str) {
    if effects.is_empty() {
        println!("  [{label}] (no effects)");
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
                println!("  [{label}] effect: %{tag_str}");
            }
        }
    }
}
