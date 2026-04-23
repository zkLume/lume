//! Graft poke builders.
//!
//! One module per primitive (settle / mint / guard / forge). Each module
//! exposes `build_<primitive>_<verb>_poke(...)` free functions that
//! return a `NounSlab` ready to hand to a kernel `poke`.
//!
//! Every commitment-bearing graft keys its trellis on `hull=@`, so the
//! signatures are uniform: `(hull, ...verb-specific...) -> NounSlab`.
//! See vesl/docs/graft-manifest.md and the parameterization plan
//! (vesl-nockup/.dev/00_PARAMETIZATION.md, §"Rust helper surface")
//! for the full convention.

pub mod settle;
pub mod mint;
pub mod guard;
pub mod forge;
