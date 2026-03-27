//! Document ingestion — read text files, split into chunks, persist as JSON.
//!
//! Phase 2.1 of the DEV.md roadmap. Replaces the hardcoded `ingest_documents()`
//! with real file-based ingestion.
//!
//! # Chunking Strategy
//!
//! Simple paragraph splitting (not embeddings — that's Phase 5 proprietary work):
//! 1. Split on double newlines (`\n\n`) to get paragraphs.
//! 2. Skip empty paragraphs (whitespace-only).
//! 3. Each non-empty paragraph becomes one chunk.
//! 4. Chunk IDs are assigned sequentially starting from 0 across all files.
//!
//! # Persistence
//!
//! The chunk store is serialized as JSON to `<output_dir>/chunk_store.json`.
//! The Merkle tree root is logged alongside it. The tree itself is rebuilt
//! from chunk data on load (deterministic — same chunks = same root).

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::merkle::MerkleTree;
use crate::types::Chunk;

/// Metadata about the ingestion run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngestMeta {
    /// Source directory that was ingested.
    pub source_dir: String,
    /// Number of files processed.
    pub file_count: usize,
    /// Total chunks produced.
    pub chunk_count: usize,
    /// Merkle root (hex-encoded) over all chunk data.
    pub merkle_root: String,
}

/// Persisted chunk store — chunks + ingestion metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkStore {
    pub meta: IngestMeta,
    pub chunks: Vec<Chunk>,
}

impl ChunkStore {
    /// Rebuild the Merkle tree from stored chunks.
    pub fn build_tree(&self) -> MerkleTree {
        let leaf_data: Vec<&[u8]> = self.chunks.iter().map(|c| c.dat.as_bytes()).collect();
        MerkleTree::build(&leaf_data)
    }

    /// Save the chunk store as JSON to `path`.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        fs::write(path, json)
    }

    /// Load a chunk store from a JSON file.
    pub fn load(path: &Path) -> std::io::Result<Self> {
        let json = fs::read_to_string(path)?;
        serde_json::from_str(&json)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))
    }
}

/// Split a text document into paragraph chunks.
///
/// Splits on double newlines (`\n\n`). Trims each paragraph and skips
/// empty ones. Returns the raw paragraph strings.
fn split_paragraphs(text: &str) -> Vec<String> {
    text.split("\n\n")
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .collect()
}

/// Collect text files from a directory (non-recursive, `.txt` extension).
///
/// Returns paths sorted alphabetically for deterministic chunk ordering.
fn collect_text_files(dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut files: Vec<PathBuf> = fs::read_dir(dir)?
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let path = entry.path();
            if path.is_file() && path.extension().and_then(|e| e.to_str()) == Some("txt") {
                Some(path)
            } else {
                None
            }
        })
        .collect();
    files.sort();
    Ok(files)
}

/// Ingest all `.txt` files from `dir`, split into paragraph chunks, build
/// a Merkle tree, and return the chunk store.
///
/// Chunk IDs are assigned sequentially starting from 0 across all files.
/// Files are processed in alphabetical order for deterministic output.
pub fn ingest_directory(dir: &Path) -> std::io::Result<ChunkStore> {
    let files = collect_text_files(dir)?;

    let mut chunks: Vec<Chunk> = Vec::new();
    let mut next_id: u64 = 0;

    for file_path in &files {
        let text = fs::read_to_string(file_path)?;
        let paragraphs = split_paragraphs(&text);

        for para in paragraphs {
            chunks.push(Chunk {
                id: next_id,
                dat: para,
            });
            next_id += 1;
        }
    }

    if chunks.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "no text chunks found in directory",
        ));
    }

    let leaf_data: Vec<&[u8]> = chunks.iter().map(|c| c.dat.as_bytes()).collect();
    let tree = MerkleTree::build(&leaf_data);
    let root_hex = crate::merkle::format_tip5(&tree.root());

    let meta = IngestMeta {
        source_dir: dir.to_string_lossy().into_owned(),
        file_count: files.len(),
        chunk_count: chunks.len(),
        merkle_root: root_hex,
    };

    Ok(ChunkStore { meta, chunks })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    /// Helper: create a temp dir with text files.
    fn setup_docs(files: &[(&str, &str)]) -> TempDir {
        let dir = TempDir::new().unwrap();
        for (name, content) in files {
            fs::write(dir.path().join(name), content).unwrap();
        }
        dir
    }

    #[test]
    fn ingest_single_file() {
        let dir = setup_docs(&[("doc.txt", "First paragraph.\n\nSecond paragraph.")]);
        let store = ingest_directory(dir.path()).unwrap();

        assert_eq!(store.meta.file_count, 1);
        assert_eq!(store.meta.chunk_count, 2);
        assert_eq!(store.chunks[0].id, 0);
        assert_eq!(store.chunks[0].dat, "First paragraph.");
        assert_eq!(store.chunks[1].id, 1);
        assert_eq!(store.chunks[1].dat, "Second paragraph.");
    }

    #[test]
    fn ingest_multiple_files_alphabetical() {
        let dir = setup_docs(&[
            ("b_second.txt", "From file B."),
            ("a_first.txt", "From file A."),
        ]);
        let store = ingest_directory(dir.path()).unwrap();

        assert_eq!(store.meta.file_count, 2);
        assert_eq!(store.chunks[0].dat, "From file A.");
        assert_eq!(store.chunks[1].dat, "From file B.");
    }

    #[test]
    fn skips_empty_paragraphs() {
        let dir = setup_docs(&[("doc.txt", "Keep this.\n\n\n\n\n\nKeep this too.")]);
        let store = ingest_directory(dir.path()).unwrap();

        assert_eq!(store.meta.chunk_count, 2);
    }

    #[test]
    fn skips_non_txt_files() {
        let dir = setup_docs(&[
            ("doc.txt", "Valid chunk."),
            ("image.png", "not a text file"),
            ("notes.md", "also not txt"),
        ]);
        let store = ingest_directory(dir.path()).unwrap();

        assert_eq!(store.meta.file_count, 1);
        assert_eq!(store.meta.chunk_count, 1);
    }

    #[test]
    fn empty_directory_errors() {
        let dir = TempDir::new().unwrap();
        let result = ingest_directory(dir.path());
        assert!(result.is_err());
    }

    #[test]
    fn chunk_ids_sequential_across_files() {
        let dir = setup_docs(&[
            ("a.txt", "A1\n\nA2\n\nA3"),
            ("b.txt", "B1\n\nB2"),
        ]);
        let store = ingest_directory(dir.path()).unwrap();

        let ids: Vec<u64> = store.chunks.iter().map(|c| c.id).collect();
        assert_eq!(ids, vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn merkle_root_is_deterministic() {
        let dir = setup_docs(&[("doc.txt", "Chunk one.\n\nChunk two.\n\nChunk three.")]);
        let store1 = ingest_directory(dir.path()).unwrap();
        let store2 = ingest_directory(dir.path()).unwrap();

        assert_eq!(store1.meta.merkle_root, store2.meta.merkle_root);
    }

    #[test]
    fn json_round_trip() {
        let dir = setup_docs(&[("doc.txt", "Round trip test.\n\nSecond chunk.")]);
        let store = ingest_directory(dir.path()).unwrap();

        let out_dir = TempDir::new().unwrap();
        let json_path = out_dir.path().join("chunk_store.json");

        store.save(&json_path).unwrap();
        let loaded = ChunkStore::load(&json_path).unwrap();

        assert_eq!(loaded.meta.chunk_count, store.meta.chunk_count);
        assert_eq!(loaded.meta.merkle_root, store.meta.merkle_root);
        assert_eq!(loaded.chunks.len(), store.chunks.len());
        for (a, b) in loaded.chunks.iter().zip(store.chunks.iter()) {
            assert_eq!(a.id, b.id);
            assert_eq!(a.dat, b.dat);
        }
    }

    #[test]
    fn rebuild_tree_matches_original() {
        let dir = setup_docs(&[("doc.txt", "Alpha.\n\nBravo.\n\nCharlie.\n\nDelta.")]);
        let store = ingest_directory(dir.path()).unwrap();

        let tree = store.build_tree();
        assert_eq!(crate::merkle::format_tip5(&tree.root()), store.meta.merkle_root);
    }
}
