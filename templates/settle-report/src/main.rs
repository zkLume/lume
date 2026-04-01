use std::error::Error;
use std::fs;

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

    let kernel = fs::read("out.jam")
        .map_err(|e| format!("Failed to read out.jam: {}", e))?;

    let mut app: NockApp =
        boot::setup(&kernel, Some(cli), &[], "{{project_name}}", None).await?;

    // Step 1: Commit data for ID 1
    // The kernel stores shax(data) as the commitment hash
    let mut slab = NounSlab::new();
    let poke = T(&mut slab, &[
        D(tas!(b"commit")),
        D(1),                // id
        D(42),               // data (kernel stores shax(42))
    ]);
    slab.set_root(poke);

    println!("--- step 1: commit data for id=1 ---");
    let effects = app.poke(SystemWire.to_wire(), slab).await?;
    print_effects(&effects);

    // Step 2: Settle with the same data (should succeed)
    // shax(42) matches the commitment
    let mut slab = NounSlab::new();
    let poke = T(&mut slab, &[
        D(tas!(b"settle")),
        D(1),                // same id
        D(42),               // same data — hash matches
    ]);
    slab.set_root(poke);

    println!("\n--- step 2: settle id=1 with correct data ---");
    let effects = app.poke(SystemWire.to_wire(), slab).await?;
    print_effects(&effects);

    // Step 3: Try to settle again (replay — should reject)
    let mut slab = NounSlab::new();
    let poke = T(&mut slab, &[
        D(tas!(b"settle")),
        D(1),                // same id again
        D(42),
    ]);
    slab.set_root(poke);

    println!("\n--- step 3: replay settle id=1 (should reject) ---");
    let effects = app.poke(SystemWire.to_wire(), slab).await?;
    print_effects(&effects);

    // Step 4: Try to settle uncommitted ID (should reject)
    let mut slab = NounSlab::new();
    let poke = T(&mut slab, &[
        D(tas!(b"settle")),
        D(999),              // no commitment for this id
        D(42),
    ]);
    slab.set_root(poke);

    println!("\n--- step 4: settle id=999 (no commitment) ---");
    let effects = app.poke(SystemWire.to_wire(), slab).await?;
    print_effects(&effects);

    // Step 5: Commit + settle with wrong data (should reject)
    let mut slab = NounSlab::new();
    let poke = T(&mut slab, &[
        D(tas!(b"commit")),
        D(2),                // new id
        D(100),              // commit hash of 100
    ]);
    slab.set_root(poke);

    println!("\n--- step 5a: commit id=2 with data=100 ---");
    let effects = app.poke(SystemWire.to_wire(), slab).await?;
    print_effects(&effects);

    let mut slab = NounSlab::new();
    let poke = T(&mut slab, &[
        D(tas!(b"settle")),
        D(2),                // same id
        D(999),              // wrong data — shax(999) != shax(100)
    ]);
    slab.set_root(poke);

    println!("--- step 5b: settle id=2 with wrong data (should reject) ---");
    let effects = app.poke(SystemWire.to_wire(), slab).await?;
    print_effects(&effects);

    Ok(())
}

fn print_effects(effects: &[NounSlab]) {
    if effects.is_empty() {
        println!("  (no effects — kernel nacked)");
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
                println!("  effect: %{}", tag_str);
            }
        }
    }
}
