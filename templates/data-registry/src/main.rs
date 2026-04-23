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
        boot::setup(&kernel, cli, &[], "data-registry", None).await?;

    // Register data under name "doc-v1"
    // The kernel hashes the data with SHA-256 and stores the hash
    let mut slab = NounSlab::new();
    let poke = T(&mut slab, &[
        D(tas!(b"register")),
        D(tas!(b"doc-v1")),      // name (@t)
        D(0xCAFEBABE),           // data (@)
    ]);
    slab.set_root(poke);

    println!("--- registering 'doc-v1' ---");
    let effects = app.poke(SystemWire.to_wire(), slab).await?;
    print_effects(&effects);

    // Verify with the same data (should pass)
    let mut slab = NounSlab::new();
    let poke = T(&mut slab, &[
        D(tas!(b"verify")),
        D(tas!(b"doc-v1")),
        D(0xCAFEBABE),           // same data
    ]);
    slab.set_root(poke);

    println!("\n--- verifying 'doc-v1' with correct data ---");
    let effects = app.poke(SystemWire.to_wire(), slab).await?;
    print_effects(&effects);

    // Verify with different data (should fail)
    let mut slab = NounSlab::new();
    let poke = T(&mut slab, &[
        D(tas!(b"verify")),
        D(tas!(b"doc-v1")),
        D(0xDEADBEEF),           // different data
    ]);
    slab.set_root(poke);

    println!("\n--- verifying 'doc-v1' with wrong data ---");
    let effects = app.poke(SystemWire.to_wire(), slab).await?;
    print_effects(&effects);

    // Look up a registered hash
    let mut slab = NounSlab::new();
    let poke = T(&mut slab, &[
        D(tas!(b"lookup")),
        D(tas!(b"doc-v1")),
    ]);
    slab.set_root(poke);

    println!("\n--- looking up 'doc-v1' ---");
    let effects = app.poke(SystemWire.to_wire(), slab).await?;
    print_effects(&effects);

    // Look up a name that doesn't exist
    let mut slab = NounSlab::new();
    let poke = T(&mut slab, &[
        D(tas!(b"lookup")),
        D(tas!(b"ghost")),
    ]);
    slab.set_root(poke);

    println!("\n--- looking up 'ghost' (not registered) ---");
    let effects = app.poke(SystemWire.to_wire(), slab).await?;
    print_effects(&effects);

    Ok(())
}

fn print_effects(effects: &[NounSlab]) {
    if effects.is_empty() {
        println!("  (no effects)");
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
