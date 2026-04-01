//! Ergonomic Nock noun construction from Rust.
//!
//! Provides helpers for building Nock nouns without wrestling the raw
//! NockStack / NounSlab APIs directly. Covers the patterns every NockApp
//! needs: atoms, cords, tags, loobeans, lists, cells, and jam/cue.
//!
//! # Quick Start
//!
//! ```ignore
//! use nock_noun_rs::*;
//!
//! let mut stack = new_stack();
//!
//! // Build a tagged cell: [%hello 'world']
//! let tag  = make_tag(&mut stack, "hello");
//! let body = make_cord(&mut stack, "world");
//! let cell = T(&mut stack, &[tag, body]);
//!
//! // Jam to bytes for wire transmission
//! let bytes = jam_to_bytes(&mut stack, cell);
//! ```
//!
//! # Memory Model
//!
//! NockStack is an arena allocator with two stacks growing toward each other.
//! All noun allocations are bump-allocated on the current frame.
//! We allocate a single stack at the start and pass `&mut stack` through
//! every builder function. No frame push/pop needed for simple noun
//! construction — build the entire noun tree in a single pass.
//!
//! # Nock Encoding Conventions
//!
//! - **Loobeans**: Hoon `%.y` (true) = atom `0`, `%.n` (false) = atom `1`.
//!   Rust `true` maps to `D(0)`, Rust `false` maps to `D(1)`.
//! - **Cords (@t)**: UTF-8 bytes of the string are the LE bytes of the atom.
//!   `"alpha"` -> atom from bytes `[97,108,112,104,97]`.
//! - **Lists**: Null-terminated right-leaning: `[a [b [c 0]]]`. Empty = `D(0)`.
//! - **Tags (@tas)**: ASCII string as atom. `%pending` = atom from `b"pending"`.

// Re-exports — the essentials every NockApp needs.
pub use nockapp::noun::slab::NounSlab;
pub use nockvm::mem::NockStack;
pub use nockvm::noun::{Cell, D, IndirectAtom, Noun, NounAllocator, T};
pub use nockvm::serialization::{cue, jam};

/// Default NockStack size: 8 MB (in 64-bit words).
const STACK_SIZE: usize = 1 << 20;

/// Create a NockStack for noun construction and jamming.
///
/// 8 MB arena — sufficient for most NockApp payloads.
/// For very large nouns, use `NockStack::new(size, 0)` directly.
pub fn new_stack() -> NockStack {
    NockStack::new(STACK_SIZE, 0)
}

// ---------------------------------------------------------------------------
// NockStack-based builders
// ---------------------------------------------------------------------------

/// Convert a byte slice to a Nock atom (LE interpretation).
///
/// Empty slice returns `D(0)` (null atom).
///
/// ```ignore
/// let noun = make_atom(&mut stack, &[0x42, 0x00, 0x01]);
/// ```
pub fn make_atom(stack: &mut NockStack, bytes: &[u8]) -> Noun {
    if bytes.is_empty() {
        return D(0);
    }
    unsafe {
        let mut indirect = IndirectAtom::new_raw_bytes_ref(stack, bytes);
        indirect.normalize_as_atom().as_noun()
    }
}

/// Convert a Rust string to a Hoon cord (`@t`).
///
/// UTF-8 bytes become the LE representation of the atom.
/// `"abc"` -> atom 6513249 (= 97 + 98*256 + 99*65536).
pub fn make_cord(stack: &mut NockStack, s: &str) -> Noun {
    make_atom(stack, s.as_bytes())
}

/// Convert an ASCII string to a Hoon tag (`@tas`).
///
/// Identical to `make_cord` for pure-ASCII inputs but semantically
/// distinct: tags are knots (`%foo`), cords are text (`'foo'`).
pub fn make_tag(stack: &mut NockStack, s: &str) -> Noun {
    make_atom(stack, s.as_bytes())
}

/// Convert a Rust bool to a Hoon loobean.
///
/// `true` (`%.y`) -> `D(0)`, `false` (`%.n`) -> `D(1)`.
///
/// This is the opposite of C/Rust convention. Hoon uses `0` for
/// true because it's "yes" / `%.y` — the first in the union.
pub fn make_loobean(b: bool) -> Noun {
    if b { D(0) } else { D(1) }
}

/// Build a null-terminated list from a slice of nouns.
///
/// `[a [b [c 0]]]` — the standard Hoon list encoding.
/// Empty slice returns `D(0)` (null / `~`).
pub fn make_list(stack: &mut NockStack, items: &[Noun]) -> Noun {
    let mut list = D(0);
    for item in items.iter().rev() {
        list = Cell::new(stack, *item, list).as_noun();
    }
    list
}

/// Jam a noun into bytes for wire transmission.
///
/// Returns the minimal LE byte representation of the jammed atom.
/// This is the format Hoon's `cue` expects.
pub fn jam_to_bytes(stack: &mut NockStack, noun: Noun) -> Vec<u8> {
    let atom = jam(stack, noun);
    let bytes = atom.as_ne_bytes();
    let len = bytes.iter().rposition(|&b| b != 0).map_or(0, |pos| pos + 1);
    bytes[..len].to_vec()
}

/// Cue bytes back into a noun.
///
/// Inverse of `jam_to_bytes`. Returns `None` if the bytes are not
/// a valid jammed noun.
pub fn cue_from_bytes(stack: &mut NockStack, bytes: &[u8]) -> Option<Noun> {
    let atom = make_atom(stack, bytes);
    let a = atom.as_atom().ok()?;
    cue(stack, a).ok()
}

// ---------------------------------------------------------------------------
// Generic-allocator builders — work with NockStack or NounSlab
// ---------------------------------------------------------------------------

/// Convert a byte slice to a Nock atom using any allocator.
///
/// Use this with `NounSlab` for poke construction:
/// ```ignore
/// let mut slab = NounSlab::new();
/// let tag = make_atom_in(&mut slab, b"settle");
/// ```
pub fn make_atom_in(alloc: &mut impl NounAllocator, bytes: &[u8]) -> Noun {
    if bytes.is_empty() {
        return D(0);
    }
    unsafe {
        let mut indirect = IndirectAtom::new_raw_bytes_ref(alloc, bytes);
        indirect.normalize_as_atom().as_noun()
    }
}

/// Convert a string to a cord using any allocator.
pub fn make_cord_in(alloc: &mut impl NounAllocator, s: &str) -> Noun {
    make_atom_in(alloc, s.as_bytes())
}

/// Convert an ASCII string to a tag using any allocator.
pub fn make_tag_in(alloc: &mut impl NounAllocator, s: &str) -> Noun {
    make_atom_in(alloc, s.as_bytes())
}

/// Build a null-terminated list using any allocator.
pub fn make_list_in(alloc: &mut impl NounAllocator, items: &[Noun]) -> Noun {
    let mut list = D(0);
    for item in items.iter().rev() {
        list = Cell::new(alloc, *item, list).as_noun();
    }
    list
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loobean_encoding() {
        // %.y (true) = 0, %.n (false) = 1
        assert_eq!(make_loobean(true).as_atom().unwrap().as_u64().unwrap(), 0);
        assert_eq!(make_loobean(false).as_atom().unwrap().as_u64().unwrap(), 1);
    }

    #[test]
    fn cord_encoding() {
        let mut stack = new_stack();
        // 'abc' in Hoon = 97 + 98*256 + 99*65536 = 6513249
        let abc = make_cord(&mut stack, "abc");
        let val = abc.as_atom().unwrap().as_u64().unwrap();
        assert_eq!(val, 97 + 98 * 256 + 99 * 65536);
    }

    #[test]
    fn tag_encoding() {
        let mut stack = new_stack();
        // %pending = atom from bytes "pending"
        let tag = make_tag(&mut stack, "pending");
        let expected: u64 = b"pending"
            .iter()
            .enumerate()
            .map(|(i, &b)| (b as u64) << (i * 8))
            .sum();
        let val = tag.as_atom().unwrap().as_u64().unwrap();
        assert_eq!(val, expected);
    }

    #[test]
    fn empty_atom() {
        let mut stack = new_stack();
        let noun = make_atom(&mut stack, &[]);
        assert_eq!(noun.as_atom().unwrap().as_u64().unwrap(), 0);
    }

    #[test]
    fn list_structure() {
        let mut stack = new_stack();
        // [1 [2 [3 0]]]
        let items = [D(1), D(2), D(3)];
        let list = make_list(&mut stack, &items);

        let c1 = list.as_cell().unwrap();
        assert_eq!(c1.head().as_atom().unwrap().as_u64().unwrap(), 1);

        let c2 = c1.tail().as_cell().unwrap();
        assert_eq!(c2.head().as_atom().unwrap().as_u64().unwrap(), 2);

        let c3 = c2.tail().as_cell().unwrap();
        assert_eq!(c3.head().as_atom().unwrap().as_u64().unwrap(), 3);

        // Null terminator
        assert_eq!(c3.tail().as_atom().unwrap().as_u64().unwrap(), 0);
    }

    #[test]
    fn empty_list() {
        let mut stack = new_stack();
        let list = make_list(&mut stack, &[]);
        assert_eq!(list.as_atom().unwrap().as_u64().unwrap(), 0);
    }

    #[test]
    fn jam_cue_round_trip() {
        let mut stack = new_stack();
        let tag = make_tag(&mut stack, "hello");
        let body = make_cord(&mut stack, "world");
        let cell = T(&mut stack, &[tag, body]);

        let bytes = jam_to_bytes(&mut stack, cell);
        assert!(!bytes.is_empty());

        let restored = cue_from_bytes(&mut stack, &bytes).expect("cue must succeed");
        assert!(restored.is_cell());
    }

    #[test]
    fn slab_builders() {
        let mut slab: NounSlab = NounSlab::new();
        let tag = make_tag_in(&mut slab, "settle");
        let payload = make_atom_in(&mut slab, &[0x42, 0x01]);
        let cause = T(&mut slab, &[tag, payload]);
        slab.set_root(cause);
        // Just verify it doesn't panic — slab nouns can't be inspected
        // the same way as stack nouns without converting.
    }
}
