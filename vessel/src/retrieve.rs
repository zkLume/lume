//! Retrieval — given a query and a chunk store, return Top-K chunks with scores.
//!
//! Phase 2.3 of the DEV.md roadmap. Provides:
//! - `Retriever` trait for swappable retrieval backends
//! - `KeywordRetriever` — simple term-frequency scoring (placeholder)
//!
//! # Why not embeddings now?
//!
//! Embedding models add a heavy dependency (ONNX runtime or API calls).
//! The ZK circuit doesn't verify retrieval quality — it only verifies that
//! retrieved chunks exist in the committed Merkle tree. The retrieval
//! algorithm is off-chain and not provable. Getting the pipeline working
//! with keyword retrieval validates the architecture without the complexity.
//!
//! Production embedding-based retrieval with proprietary ranking is Phase 5.
//!
//! # Score Convention
//!
//! Scores are returned as `f64` in [0.0, 1.0] by the `Retriever` trait,
//! then converted to Hoon-compatible `@ud` fixed-point (×10^6) at the
//! call site. ZK arithmetic is integer-only.

use crate::types::Chunk;

/// Fixed-point multiplier: scores are stored as `@ud` = score × 10^6.
pub const SCORE_SCALE: u64 = 1_000_000;

// ---------------------------------------------------------------------------
// Retriever trait
// ---------------------------------------------------------------------------

/// Pluggable retrieval backend.
///
/// Given a query and a set of chunks, returns the top-K most relevant
/// chunk indices with similarity scores in [0.0, 1.0].
///
/// Results are returned sorted by score descending (most relevant first).
pub trait Retriever {
    fn retrieve(&self, query: &str, chunks: &[Chunk], k: usize) -> Vec<RetrievalHit>;
}

/// A single retrieval result: chunk index + relevance score.
#[derive(Debug, Clone)]
pub struct RetrievalHit {
    /// Index into the chunk store's `chunks` vec.
    pub chunk_index: usize,
    /// Relevance score in [0.0, 1.0].
    pub score: f64,
}

impl RetrievalHit {
    /// Convert the floating-point score to Hoon-compatible fixed-point `@ud`.
    pub fn score_fixed(&self) -> u64 {
        (self.score * SCORE_SCALE as f64) as u64
    }
}

// ---------------------------------------------------------------------------
// KeywordRetriever — term-frequency scoring
// ---------------------------------------------------------------------------

/// Simple keyword retriever using term-frequency scoring.
///
/// Algorithm:
/// 1. Tokenize query into lowercase words (split on whitespace + punctuation).
/// 2. For each chunk, count how many query terms appear (case-insensitive).
/// 3. Score = matching_terms / total_query_terms (normalized to [0.0, 1.0]).
/// 4. Sort by score descending, return top K with score > 0.
///
/// This is a placeholder for Phase 5's embedding-based retrieval. It validates
/// the pipeline architecture without introducing model dependencies.
pub struct KeywordRetriever;

impl KeywordRetriever {
    /// Tokenize text into lowercase words, stripping common punctuation.
    fn tokenize(text: &str) -> Vec<String> {
        text.split(|c: char| c.is_whitespace() || matches!(c, ',' | '.' | ':' | ';' | '!' | '?' | '(' | ')' | '"' | '\''))
            .map(|w| w.to_lowercase())
            .filter(|w| !w.is_empty())
            .collect()
    }
}

impl Retriever for KeywordRetriever {
    fn retrieve(&self, query: &str, chunks: &[Chunk], k: usize) -> Vec<RetrievalHit> {
        let query_terms = Self::tokenize(query);
        if query_terms.is_empty() {
            return vec![];
        }

        let total = query_terms.len() as f64;

        let mut hits: Vec<RetrievalHit> = chunks
            .iter()
            .enumerate()
            .filter_map(|(idx, chunk)| {
                let chunk_lower = chunk.dat.to_lowercase();
                let matching = query_terms
                    .iter()
                    .filter(|term| chunk_lower.contains(term.as_str()))
                    .count();

                if matching > 0 {
                    Some(RetrievalHit {
                        chunk_index: idx,
                        score: matching as f64 / total,
                    })
                } else {
                    None
                }
            })
            .collect();

        // Sort by score descending, then by chunk index for determinism
        hits.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.chunk_index.cmp(&b.chunk_index))
        });

        hits.truncate(k);
        hits
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn enterprise_chunks() -> Vec<Chunk> {
        vec![
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
        ]
    }

    #[test]
    fn keyword_retriever_ranks_by_relevance() {
        let retriever = KeywordRetriever;
        let chunks = enterprise_chunks();
        let hits = retriever.retrieve("Q3 revenue growth", &chunks, 4);

        // Chunk 0 has "Q3", "revenue", "growth" — 3/3 match
        assert!(!hits.is_empty());
        assert_eq!(hits[0].chunk_index, 0);
        assert!((hits[0].score - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn keyword_retriever_respects_k_limit() {
        let retriever = KeywordRetriever;
        let chunks = enterprise_chunks();
        let hits = retriever.retrieve("Q3 Q4 revenue audit", &chunks, 2);

        assert!(hits.len() <= 2);
    }

    #[test]
    fn keyword_retriever_case_insensitive() {
        let retriever = KeywordRetriever;
        let chunks = enterprise_chunks();

        let hits_lower = retriever.retrieve("soc2 audit", &chunks, 4);
        let hits_upper = retriever.retrieve("SOC2 AUDIT", &chunks, 4);

        assert_eq!(hits_lower.len(), hits_upper.len());
        assert_eq!(hits_lower[0].chunk_index, hits_upper[0].chunk_index);
    }

    #[test]
    fn keyword_retriever_no_match_returns_empty() {
        let retriever = KeywordRetriever;
        let chunks = enterprise_chunks();
        let hits = retriever.retrieve("quantum computing blockchain", &chunks, 4);

        assert!(hits.is_empty());
    }

    #[test]
    fn keyword_retriever_empty_query_returns_empty() {
        let retriever = KeywordRetriever;
        let chunks = enterprise_chunks();
        let hits = retriever.retrieve("", &chunks, 4);

        assert!(hits.is_empty());
    }

    #[test]
    fn keyword_retriever_empty_chunks_returns_empty() {
        let retriever = KeywordRetriever;
        let hits = retriever.retrieve("some query", &[], 4);

        assert!(hits.is_empty());
    }

    #[test]
    fn keyword_retriever_partial_match_scoring() {
        let retriever = KeywordRetriever;
        let chunks = enterprise_chunks();
        // "risk" matches chunk 1, "revenue" matches chunk 0
        // Neither chunk matches both terms → both score 0.5
        let hits = retriever.retrieve("risk revenue", &chunks, 4);

        assert_eq!(hits.len(), 2);
        // Both have score 0.5, sorted by chunk_index for determinism
        assert!((hits[0].score - 0.5).abs() < f64::EPSILON);
        assert!((hits[1].score - 0.5).abs() < f64::EPSILON);
        assert_eq!(hits[0].chunk_index, 0);
        assert_eq!(hits[1].chunk_index, 1);
    }

    #[test]
    fn score_fixed_conversion() {
        let hit = RetrievalHit {
            chunk_index: 0,
            score: 0.95,
        };
        assert_eq!(hit.score_fixed(), 950_000);

        let hit_full = RetrievalHit {
            chunk_index: 0,
            score: 1.0,
        };
        assert_eq!(hit_full.score_fixed(), 1_000_000);

        let hit_zero = RetrievalHit {
            chunk_index: 0,
            score: 0.0,
        };
        assert_eq!(hit_zero.score_fixed(), 0);
    }

    #[test]
    fn keyword_retriever_sorted_descending() {
        let retriever = KeywordRetriever;
        let chunks = enterprise_chunks();
        // "Q3 growth" → chunk 0 scores 2/2=1.0, chunk 3 has "Q4" not "Q3"
        let hits = retriever.retrieve("Q3 growth", &chunks, 4);

        // Results must be sorted descending by score
        for window in hits.windows(2) {
            assert!(window[0].score >= window[1].score);
        }
    }
}
