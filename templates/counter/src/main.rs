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

    // Increment 3 times
    for i in 0..3 {
        let mut slab = NounSlab::new();
        let poke = T(&mut slab, &[D(tas!(b"inc")), D(0)]);
        slab.set_root(poke);

        let effects = app.poke(SystemWire.to_wire(), slab).await?;
        print_effects(&effects, &format!("inc #{}", i + 1));
    }

    // Decrement once
    let mut slab = NounSlab::new();
    let poke = T(&mut slab, &[D(tas!(b"dec")), D(0)]);
    slab.set_root(poke);

    let effects = app.poke(SystemWire.to_wire(), slab).await?;
    print_effects(&effects, "dec");

    // Reset
    let mut slab = NounSlab::new();
    let poke = T(&mut slab, &[D(tas!(b"reset")), D(0)]);
    slab.set_root(poke);

    let effects = app.poke(SystemWire.to_wire(), slab).await?;
    print_effects(&effects, "reset");

    Ok(())
}

fn print_effects(effects: &[NounSlab], label: &str) {
    for effect in effects.iter() {
        let noun = unsafe { effect.root() };
        if let Ok(cell) = noun.as_cell() {
            if cell.head().eq_bytes(b"count") {
                if let Ok(atom) = cell.tail().as_atom() {
                    println!("[{}] count = {}", label, atom.as_u64().unwrap_or(0));
                }
            }
        }
    }
}
