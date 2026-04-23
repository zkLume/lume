use std::error::Error;
use std::fs;

use nock_noun_rs::{make_atom_in, make_tag_in};
use nockapp::kernel::boot;
use nockapp::noun::slab::NounSlab;
use nockapp::wire::{SystemWire, Wire};
use nockapp::NockApp;
use nockvm::noun::{D, T};

/// graft-intent — family-5 placeholder driver.
///
/// Pokes `%intent-declare` to prove the placeholder crashes loudly with
/// `%intent-graft-placeholder`. A working intent coordination app will
/// drop into this same shape once Nockchain upstream publishes the
/// canonical intent structure; until then, this binary exists to
/// demonstrate the reservation, not to do any work.
#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let cli = boot::default_boot_cli(false);
    boot::init_default_tracing(&cli);

    let kernel =
        fs::read("out.jam").map_err(|e| format!("Failed to read out.jam: {}", e))?;

    let mut app: NockApp =
        boot::setup(&kernel, cli, &[], "graft-intent", None).await?;

    println!("=== graft-intent placeholder demo ===");
    println!();
    println!("Poking %intent-declare on the placeholder kernel.");
    println!("Expected: kernel crashes with %intent-graft-placeholder.");
    println!();

    let mut slab = NounSlab::new();
    let tag = make_tag_in(&mut slab, "intent-declare");
    let hull = D(1);
    let body = make_atom_in(&mut slab, b"placeholder-body");
    let expires_at = D(0);
    let poke = T(&mut slab, &[tag, hull, body, expires_at]);
    slab.set_root(poke);

    let result = app.poke(SystemWire.to_wire(), slab).await;
    match result {
        Ok(effects) => {
            println!("unexpected: poke returned without crashing ({} effects)", effects.len());
            println!("the placeholder is supposed to bang on every cause arm.");
            println!("check that hoon/lib/intent-graft.hoon still has the bang arms.");
            Err("placeholder did not crash".into())
        }
        Err(e) => {
            println!("kernel crashed as expected:");
            println!("  {}", e);
            println!();
            println!("the %intent-graft-placeholder trace confirms the family-5 slot");
            println!("is reserved, not implemented. swap intent-graft.hoon when the");
            println!("canonical upstream intent structure lands.");
            Ok(())
        }
    }
}
