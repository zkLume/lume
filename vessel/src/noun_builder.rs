//! Nock noun construction and jam serialization for the Lume settlement payload.
//!
//! Converts Rust types into Nock nouns matching the ABI defined in
//! `protocol/lib/lume-entrypoint.hoon:+$settlement-payload`, then jams
//! the noun into a single atom for wire transmission.
//!
//! # Memory Model
//!
//! NockStack is an arena allocator with two stacks growing toward each other.
//! All noun allocations are bump-allocated on the current frame.
//! We allocate a single stack at the start and pass `&mut stack` through
//! every builder function. No frame push/pop needed for our use case —
//! we build the entire noun tree in a single pass.
//!
//! # Nock Encoding Traps
//!
//! - **Loobeans**: Hoon `%.y` (true) = atom `0`, `%.n` (false) = atom `1`.
//!   Rust `true` maps to `D(0)`, Rust `false` maps to `D(1)`.
//! - **Cords (@t)**: UTF-8 bytes of the string are the LE bytes of the atom.
//!   `"alpha"` → atom from bytes `[97,108,112,104,97]`.
//! - **Lists**: Null-terminated right-leaning: `[a [b [c 0]]]`. Empty = `D(0)`.
//! - **Hashes (@)**: tip5 digests `[u64; 5]` encoded as flat atoms via
//!   base-p polynomial: `a + b*P + c*P^2 + d*P^3 + e*P^4` where P = Goldilocks.
//!   Matches Hoon's `digest-to-atom:tip5`.
//! - **Tags (@tas)**: ASCII string as atom. `%pending` = atom from `b"pending"`.

use nockapp::noun::slab::NounSlab;
use nockchain_math::belt::PRIME;
use nockvm::mem::NockStack;
use nockvm::noun::{Cell, D, IndirectAtom, Noun, NounAllocator, T};
use nockvm::serialization::jam;

#[cfg(test)]
use crate::merkle::MerkleTree;
use crate::types::*;

/// Convert a tip5 hash ([u64; 5]) to LE atom bytes.
///
/// Computes the base-p polynomial: `limb[0] + limb[1]*P + limb[2]*P^2 + ...`
/// where P = Goldilocks prime.  Matches Hoon's `digest-to-atom:tip5`.
///
/// Uses u128 arithmetic with carry propagation to avoid BigUint dependency.
pub fn tip5_to_atom_le_bytes(hash: &Tip5Hash) -> Vec<u8> {
    // Compute the polynomial in base-256 (byte) representation.
    // Maximum size: 5 limbs * 64 bits = 320 bits = 40 bytes.
    let mut result = [0u8; 48]; // extra room for carry
    let mut result_len: usize = 0;

    // For each limb (from high to low power), multiply running result by PRIME
    // and add the limb. This is Horner's method:
    // value = (((limb[4] * P + limb[3]) * P + limb[2]) * P + limb[1]) * P + limb[0]
    for &limb in hash.iter().rev() {
        // Multiply result by PRIME
        let mut carry: u128 = 0;
        for byte in result[..result_len].iter_mut() {
            let prod = (*byte as u128) * (PRIME as u128) + carry;
            *byte = prod as u8;
            carry = prod >> 8;
        }
        while carry > 0 {
            if result_len < result.len() {
                result[result_len] = carry as u8;
                result_len += 1;
            }
            carry >>= 8;
        }

        // Add limb
        let mut add_carry: u128 = limb as u128;
        for byte in result.iter_mut() {
            if add_carry == 0 {
                break;
            }
            let sum = (*byte as u128) + add_carry;
            *byte = sum as u8;
            add_carry = sum >> 8;
        }
        if result_len == 0 && limb > 0 {
            // Determine how many bytes the limb needs
            result_len = 8; // max for u64
        }
        // Update result_len to account for any growth
        while result_len < result.len() && result[result_len] != 0 {
            result_len += 1;
        }
    }

    // Trim trailing zeros for minimal representation
    let len = result.iter().rposition(|&b| b != 0).map_or(0, |p| p + 1);
    if len == 0 {
        return vec![0];
    }
    result[..len].to_vec()
}

/// Default NockStack size: 8 MB (in 64-bit words).
const STACK_SIZE: usize = 1 << 20;

/// Create a NockStack for noun construction and jamming.
pub fn new_stack() -> NockStack {
    NockStack::new(STACK_SIZE, 0)
}

// ---------------------------------------------------------------------------
// Atom Builders
// ---------------------------------------------------------------------------

/// Convert a byte slice to a Nock atom (LE interpretation).
/// Empty slice → `D(0)` (null atom).
fn bytes_to_noun(stack: &mut NockStack, bytes: &[u8]) -> Noun {
    if bytes.is_empty() {
        return D(0);
    }
    unsafe {
        let mut indirect = IndirectAtom::new_raw_bytes_ref(stack, bytes);
        indirect.normalize_as_atom().as_noun()
    }
}

/// Convert a Rust string to a Hoon cord (@t).
/// UTF-8 bytes become the LE representation of the atom.
fn cord(stack: &mut NockStack, s: &str) -> Noun {
    bytes_to_noun(stack, s.as_bytes())
}

/// Convert a tip5 hash `[u64; 5]` to a Nock atom.
///
/// Uses base-p polynomial encoding: `a + b*P + c*P^2 + d*P^3 + e*P^4`
/// matching Hoon's `digest-to-atom:tip5`.
fn hash_to_noun(stack: &mut NockStack, hash: &Tip5Hash) -> Noun {
    let le_bytes = tip5_to_atom_le_bytes(hash);
    bytes_to_noun(stack, &le_bytes)
}

/// Convert a Rust bool to a Hoon loobean.
/// `true` (%.y) → `D(0)`, `false` (%.n) → `D(1)`.
fn loobean(b: bool) -> Noun {
    if b { D(0) } else { D(1) }
}

// ---------------------------------------------------------------------------
// Structure Builders — mirror protocol/sur/lume.hoon
// ---------------------------------------------------------------------------

/// `+$proof-node  [hash=@ side=?]`
fn proof_node_to_noun(stack: &mut NockStack, node: &ProofNode) -> Noun {
    let h = hash_to_noun(stack, &node.hash);
    let s = loobean(node.side);
    T(stack, &[h, s])
}

/// Convert a tip5 hash to the display format used by digest-to-atom.
/// Used for the note root and expected-root fields.
fn tip5_root_to_noun(stack: &mut NockStack, root: &Tip5Hash) -> Noun {
    let le_bytes = tip5_to_atom_le_bytes(root);
    bytes_to_noun(stack, &le_bytes)
}

/// `(list proof-node)` → null-terminated right-leaning cell tree.
fn proof_list_to_noun(stack: &mut NockStack, proof: &[ProofNode]) -> Noun {
    let mut list = D(0); // null terminator
    for node in proof.iter().rev() {
        let item = proof_node_to_noun(stack, node);
        list = Cell::new(stack, item, list).as_noun();
    }
    list
}

/// `+$chunk  [id=chunk-id dat=@t]`
fn chunk_to_noun(stack: &mut NockStack, chunk: &Chunk) -> Noun {
    let id = D(chunk.id);
    let dat = cord(stack, &chunk.dat);
    T(stack, &[id, dat])
}

/// `+$retrieval  [=chunk proof=merkle-proof score=@ud]`
fn retrieval_to_noun(stack: &mut NockStack, r: &Retrieval) -> Noun {
    let c = chunk_to_noun(stack, &r.chunk);
    let p = proof_list_to_noun(stack, &r.proof);
    let s = D(r.score);
    T(stack, &[c, p, s])
}

/// `(list retrieval)` → null-terminated right-leaning.
fn retrieval_list_to_noun(stack: &mut NockStack, results: &[Retrieval]) -> Noun {
    let mut list = D(0);
    for r in results.iter().rev() {
        let item = retrieval_to_noun(stack, r);
        list = Cell::new(stack, item, list).as_noun();
    }
    list
}

/// `+$manifest  [query=@t results=(list retrieval) prompt=@t output=@t]`
fn manifest_to_noun(stack: &mut NockStack, m: &Manifest) -> Noun {
    let query = cord(stack, &m.query);
    let results = retrieval_list_to_noun(stack, &m.results);
    let prompt = cord(stack, &m.prompt);
    let output = cord(stack, &m.output);
    T(stack, &[query, results, prompt, output])
}

/// `note=[id=@ vessel=@ root=@ state=[%pending ~]]`
///
/// In Nock: `[id [vessel [root [%pending 0]]]]`
/// The `%pending` tag is the atom for the ASCII bytes "pending".
fn pending_note_to_noun(stack: &mut NockStack, note: &Note) -> Noun {
    assert!(
        matches!(note.state, NoteState::Pending),
        "settlement payload requires %pending note"
    );
    let id = D(note.id);
    let vessel = D(note.vessel);
    let root = tip5_root_to_noun(stack, &note.root);
    let tag = bytes_to_noun(stack, b"pending");
    let state = Cell::new(stack, tag, D(0)).as_noun(); // [%pending ~]
    T(stack, &[id, vessel, root, state])
}

// ---------------------------------------------------------------------------
// Settlement Payload — the ABI boundary
// ---------------------------------------------------------------------------

/// Build the complete settlement payload noun.
///
/// Matches `+$settlement-payload` from `lume-entrypoint.hoon`:
/// ```text
/// [note=[id=@ vessel=@ root=@ state=[%pending ~]]
///  mani=[query=@t results=(list ...) prompt=@t output=@t]
///  expected-root=@]
/// ```
///
/// In Nock: `[note [mani expected-root]]`
pub fn build_settlement_payload(
    stack: &mut NockStack,
    note: &Note,
    manifest: &Manifest,
    expected_root: &Tip5Hash,
) -> Noun {
    let note_noun = pending_note_to_noun(stack, note);
    let mani_noun = manifest_to_noun(stack, manifest);
    let root_noun = tip5_root_to_noun(stack, expected_root);
    T(stack, &[note_noun, mani_noun, root_noun])
}

/// Jam a noun into bytes for wire transmission.
///
/// Returns the minimal LE byte representation of the jammed atom.
/// This is the exact format the Hoon entrypoint expects via `cue`.
#[cfg(test)]
pub fn jam_to_bytes(stack: &mut NockStack, noun: Noun) -> Vec<u8> {
    let atom = jam(stack, noun);
    let bytes = atom.as_ne_bytes();
    // Trim trailing zero bytes for minimal representation
    let len = bytes.iter().rposition(|&b| b != 0).map_or(0, |pos| pos + 1);
    bytes[..len].to_vec()
}

/// Full pipeline: build settlement noun → jam → bytes.
#[cfg(test)]
pub fn serialize_settlement(
    stack: &mut NockStack,
    note: &Note,
    manifest: &Manifest,
    expected_root: &Tip5Hash,
) -> Vec<u8> {
    let payload = build_settlement_payload(stack, note, manifest, expected_root);
    jam_to_bytes(stack, payload)
}

// ---------------------------------------------------------------------------
// Generic atom builders — work with any NounAllocator (NockStack or NounSlab)
// ---------------------------------------------------------------------------

/// Convert a byte slice to a Nock atom using any allocator.
fn bytes_to_noun_generic(alloc: &mut impl NounAllocator, bytes: &[u8]) -> Noun {
    if bytes.is_empty() {
        return D(0);
    }
    unsafe {
        let mut indirect = IndirectAtom::new_raw_bytes_ref(alloc, bytes);
        indirect.normalize_as_atom().as_noun()
    }
}

/// Convert a tip5 hash to a Nock atom using any allocator.
fn hash_to_noun_generic(alloc: &mut impl NounAllocator, hash: &Tip5Hash) -> Noun {
    let le_bytes = tip5_to_atom_le_bytes(hash);
    bytes_to_noun_generic(alloc, &le_bytes)
}

// ---------------------------------------------------------------------------
// NounSlab-based poke builders — for NockApp kernel interaction
// ---------------------------------------------------------------------------

/// Build a `%settle` poke cause in a NounSlab.
///
/// Constructs the settlement payload noun, jams it, then wraps as
/// `[%settle jammed-atom]` matching the kernel's `+$cause`.
pub fn build_settle_poke(
    note: &Note,
    manifest: &Manifest,
    expected_root: &Tip5Hash,
) -> NounSlab {
    // Build and jam the settlement payload using NockStack (proven correct)
    let mut stack = new_stack();
    let payload = build_settlement_payload(&mut stack, note, manifest, expected_root);
    let jammed = jam(&mut stack, payload);
    let jam_bytes = {
        let bytes = jammed.as_ne_bytes();
        let len = bytes.iter().rposition(|&b| b != 0).map_or(0, |pos| pos + 1);
        bytes[..len].to_vec()
    };

    // Build the poke cause in a NounSlab
    let mut slab = NounSlab::new();
    let tag = bytes_to_noun_generic(&mut slab, b"settle");
    let payload_atom = bytes_to_noun_generic(&mut slab, &jam_bytes);
    let cause = T(&mut slab, &[tag, payload_atom]);
    slab.set_root(cause);
    slab
}

/// Build a `%prove` poke cause in a NounSlab.
///
/// Same payload as `%settle` but tagged `%prove`. The kernel will:
/// 1. Verify the manifest (same as %settle)
/// 2. Generate a STARK proof of the settlement computation
/// 3. Return [settled-note proof] as effects
pub fn build_prove_poke(
    note: &Note,
    manifest: &Manifest,
    expected_root: &Tip5Hash,
) -> NounSlab {
    let mut stack = new_stack();
    let payload = build_settlement_payload(&mut stack, note, manifest, expected_root);
    let jammed = jam(&mut stack, payload);
    let jam_bytes = {
        let bytes = jammed.as_ne_bytes();
        let len = bytes.iter().rposition(|&b| b != 0).map_or(0, |pos| pos + 1);
        bytes[..len].to_vec()
    };

    let mut slab = NounSlab::new();
    let tag = bytes_to_noun_generic(&mut slab, b"prove");
    let payload_atom = bytes_to_noun_generic(&mut slab, &jam_bytes);
    let cause = T(&mut slab, &[tag, payload_atom]);
    slab.set_root(cause);
    slab
}

/// Build a `%register` poke cause in a NounSlab.
///
/// Constructs `[%register vessel=@ root=@]` — the NockApp framework
/// passes `cause.input.ovum` directly to the kernel's poke handler,
/// so this is not jammed (unlike settle/prove which jam the
/// settlement-payload for the kernel to cue internally).
pub fn build_register_poke(vessel_id: u64, root: &Tip5Hash) -> NounSlab {
    let mut slab = NounSlab::new();
    let tag = bytes_to_noun_generic(&mut slab, b"register");
    let id = D(vessel_id);
    let root_noun = hash_to_noun_generic(&mut slab, root);
    let cause = T(&mut slab, &[tag, id, root_noun]);
    slab.set_root(cause);
    slab
}

// ---------------------------------------------------------------------------
// Helper: build the full scenario (for testing and main pipeline)
// ---------------------------------------------------------------------------

/// Build a complete Hedge Fund scenario: 4 chunks, retrieve 0 and 1.
/// Returns (note, manifest, root) ready for serialization.
#[cfg(test)]
pub fn build_hedge_fund_scenario() -> (Note, Manifest, Tip5Hash) {
    let chunks = vec![
        Chunk {
            id: 0,
            dat: "Q3 revenue: $4.2M ARR, 18% QoQ growth".into(),
        },
        Chunk {
            id: 1,
            dat: "Risk exposure: $800K in variable-rate instruments".into(),
        },
        Chunk {
            id: 2,
            dat: "Board approved Series B at $45M pre-money".into(),
        },
        Chunk {
            id: 3,
            dat: "SOC2 Type II audit scheduled for Q4".into(),
        },
    ];

    let leaf_data: Vec<&[u8]> = chunks.iter().map(|c| c.dat.as_bytes()).collect();
    let tree = MerkleTree::build(&leaf_data);
    let root = tree.root();

    let query = "Summarize Q3 financial position";
    let retrieved_indices = [0usize, 1];

    let retrievals: Vec<Retrieval> = retrieved_indices
        .iter()
        .map(|&i| Retrieval {
            chunk: chunks[i].clone(),
            proof: tree.proof(i),
            score: 950_000,
        })
        .collect();

    let mut prompt = query.to_string();
    for &i in &retrieved_indices {
        prompt.push('\n');
        prompt.push_str(&chunks[i].dat);
    }

    let output = format!(
        "Based on the provided documents: {} The analysis indicates positive growth trajectory.",
        retrieved_indices
            .iter()
            .map(|&i| chunks[i].dat.as_str())
            .collect::<Vec<_>>()
            .join(" | ")
    );

    let manifest = Manifest {
        query: query.to_string(),
        results: retrievals,
        prompt,
        output,
    };

    let note = Note {
        id: 42,
        vessel: 7,
        root,
        state: NoteState::Pending,
    };

    (note, manifest, root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;
    use nockvm::serialization::cue;

    #[test]
    fn jam_and_write_payload() {
        let (note, manifest, root) = build_hedge_fund_scenario();
        let mut stack = new_stack();

        // Build and jam the settlement payload
        let jam_bytes = serialize_settlement(&mut stack, &note, &manifest, &root);

        assert!(!jam_bytes.is_empty(), "jammed payload must not be empty");
        println!("JAM payload: {} bytes", jam_bytes.len());
        println!(
            "First 32 bytes: {}",
            hex::encode(&jam_bytes[..32.min(jam_bytes.len())])
        );

        // Write to tests/test_payload.jam
        let test_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
        fs::create_dir_all(&test_dir).expect("create tests dir");
        let jam_path = test_dir.join("test_payload.jam");
        fs::write(&jam_path, &jam_bytes).expect("write jam file");
        println!("Wrote {} bytes to {}", jam_bytes.len(), jam_path.display());

        // Verify file exists and has content
        let read_back = fs::read(&jam_path).expect("read jam file");
        assert_eq!(read_back, jam_bytes, "file content must match");
    }

    #[test]
    fn jam_cue_round_trip() {
        let (note, manifest, root) = build_hedge_fund_scenario();
        let mut stack = new_stack();

        // Build the payload noun
        let payload = build_settlement_payload(&mut stack, &note, &manifest, &root);

        // Jam → atom
        let jammed = jam(&mut stack, payload);

        // Cue → noun (should not error)
        let cued = cue(&mut stack, jammed).expect("cue must succeed on jammed payload");

        // The cued noun should be a cell (the outer tuple)
        assert!(cued.is_cell(), "cued noun must be a cell");

        // Verify the structure: [note [mani root]]
        let outer = cued.as_cell().expect("outer cell");
        let note_noun = outer.head();
        assert!(note_noun.is_cell(), "note must be a cell");

        let rest = outer.tail();
        assert!(rest.is_cell(), "rest [mani root] must be a cell");
    }

    #[test]
    fn loobean_encoding() {
        // %.y (true) = 0, %.n (false) = 1
        assert_eq!(loobean(true).as_atom().unwrap().as_u64().unwrap(), 0);
        assert_eq!(loobean(false).as_atom().unwrap().as_u64().unwrap(), 1);
    }

    #[test]
    fn cord_encoding() {
        let mut stack = new_stack();

        // 'abc' in Hoon = 97 + 98*256 + 99*65536 = 6513249
        let abc = cord(&mut stack, "abc");
        let val = abc.as_atom().unwrap().as_u64().unwrap();
        assert_eq!(val, 97 + 98 * 256 + 99 * 65536);
    }

    #[test]
    fn tag_pending_encoding() {
        let mut stack = new_stack();

        // %pending in Hoon = atom from bytes "pending"
        let tag = bytes_to_noun(&mut stack, b"pending");
        let expected: u64 = b"pending"
            .iter()
            .enumerate()
            .map(|(i, &b)| (b as u64) << (i * 8))
            .sum();
        let val = tag.as_atom().unwrap().as_u64().unwrap();
        assert_eq!(val, expected);
    }

    #[test]
    fn list_encoding_structure() {
        let mut stack = new_stack();

        // A 2-element list [a [b 0]] where a=1, b=2
        let proof = vec![
            ProofNode {
                hash: [0xAA; 5],
                side: true,
            },
            ProofNode {
                hash: [0xBB; 5],
                side: false,
            },
        ];
        let list = proof_list_to_noun(&mut stack, &proof);

        // list = [node0 [node1 0]]
        assert!(list.is_cell(), "list must be a cell");
        let first = list.as_cell().unwrap();
        assert!(
            first.head().is_cell(),
            "first element must be a cell [hash side]"
        );

        let rest = first.tail();
        assert!(rest.is_cell(), "rest must be a cell [node1 0]");
        let second = rest.as_cell().unwrap();
        assert!(second.head().is_cell(), "second element must be a cell");

        // Null terminator
        let term = second.tail();
        assert!(term.is_atom(), "terminator must be atom 0");
        assert_eq!(term.as_atom().unwrap().as_u64().unwrap(), 0);
    }
}
