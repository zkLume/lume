//! LLM integration — pluggable inference providers.
//!
//! Phase 2.2 of the DEV.md roadmap. Provides:
//! - `LlmProvider` trait for swappable backends
//! - `OllamaProvider` for local Ollama (`localhost:11434/api/generate`)
//! - `StubProvider` for testing / demo mode (no network required)
//! - `build_prompt()` — canonical deterministic prompt construction
//!
//! # Prompt Alignment Constraint (CRITICAL)
//!
//! Prompt construction MUST match Hoon's `++build-prompt` byte-for-byte.
//! The convention is: `query + 0x0a + chunk0.dat + 0x0a + chunk1.dat + ...`
//! (newline = `\n` = byte `0x0a`).
//!
//! The cross-runtime test (Phase 9) proved hash alignment between Rust and Hoon
//! using this exact format. Any deviation breaks Merkle settlement.

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::types::Chunk;

// ---------------------------------------------------------------------------
// Prompt construction — the ZK-critical function
// ---------------------------------------------------------------------------

/// Build the deterministic prompt from query + ordered chunk data.
///
/// Mirror of Hoon's `++build-prompt`: `query + \n + chunk0.dat + \n + chunk1.dat + ...`
///
/// This is the canonical implementation. The Hoon prover reconstructs the
/// prompt from verified chunks and compares byte-for-byte. If this function
/// produces different bytes than `++build-prompt`, settlement will fail.
pub fn build_prompt(query: &str, chunks: &[&Chunk]) -> String {
    let mut prompt = query.to_string();
    for chunk in chunks {
        prompt.push('\n');
        prompt.push_str(&chunk.dat);
    }
    prompt
}

// ---------------------------------------------------------------------------
// LlmProvider trait
// ---------------------------------------------------------------------------

/// Pluggable LLM inference backend.
///
/// Implementations must be `Send + Sync` to work in the async NockApp pipeline.
/// The `generate` method receives the fully-constructed prompt (from
/// `build_prompt`) and returns the raw model output text.
///
/// Object-safe via `Pin<Box<dyn Future>>` so providers can be used as
/// `Box<dyn LlmProvider>` for runtime selection (Ollama vs stub).
pub trait LlmProvider: Send + Sync {
    fn generate(
        &self,
        prompt: &str,
    ) -> Pin<Box<dyn Future<Output = Result<String, LlmError>> + Send + '_>>;
}

/// LLM provider errors.
#[derive(Debug)]
pub enum LlmError {
    /// HTTP or network failure.
    Request(String),
    /// Response could not be parsed.
    Parse(String),
}

impl std::fmt::Display for LlmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LlmError::Request(msg) => write!(f, "LLM request error: {msg}"),
            LlmError::Parse(msg) => write!(f, "LLM parse error: {msg}"),
        }
    }
}

impl std::error::Error for LlmError {}

// ---------------------------------------------------------------------------
// OllamaProvider — local Ollama HTTP API
// ---------------------------------------------------------------------------

/// Ollama request body for `/api/generate`.
#[derive(Serialize)]
struct OllamaRequest {
    model: String,
    prompt: String,
    stream: bool,
}

/// Ollama response body from `/api/generate` (non-streaming).
#[derive(Deserialize)]
struct OllamaResponse {
    response: String,
}

/// Default generation timeout (5 minutes). Remote LLMs (e.g. RunPod) may
/// need significant time for large prompts.
const DEFAULT_GENERATE_TIMEOUT: Duration = Duration::from_secs(300);

/// LLM provider backed by an Ollama instance (local or remote).
///
/// Default endpoint: `http://localhost:11434`
/// Default model: `llama3.2`
///
/// Uses non-streaming mode (`stream: false`) so we get the full response
/// in a single JSON object. Streaming would complicate prompt-hash alignment
/// (we need the complete output for the manifest).
pub struct OllamaProvider {
    pub base_url: String,
    pub model: String,
    client: reqwest::Client,
}

impl OllamaProvider {
    pub fn new(base_url: &str, model: &str) -> Self {
        Self::with_timeout(base_url, model, DEFAULT_GENERATE_TIMEOUT)
    }

    pub fn with_timeout(base_url: &str, model: &str, timeout: Duration) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            model: model.to_string(),
            client: reqwest::Client::builder()
                .timeout(timeout)
                .build()
                .expect("reqwest client must build"),
        }
    }
}

impl Default for OllamaProvider {
    fn default() -> Self {
        Self::new("http://localhost:11434", "llama3.2")
    }
}

impl LlmProvider for OllamaProvider {
    fn generate(
        &self,
        prompt: &str,
    ) -> Pin<Box<dyn Future<Output = Result<String, LlmError>> + Send + '_>> {
        let url = format!("{}/api/generate", self.base_url);
        let body = OllamaRequest {
            model: self.model.clone(),
            prompt: prompt.to_string(),
            stream: false,
        };

        Box::pin(async move {
            let resp = self
                .client
                .post(&url)
                .json(&body)
                .send()
                .await
                .map_err(|e| LlmError::Request(e.to_string()))?;

            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                return Err(LlmError::Request(format!(
                    "Ollama returned {status}: {text}"
                )));
            }

            let parsed: OllamaResponse = resp
                .json()
                .await
                .map_err(|e| LlmError::Parse(e.to_string()))?;

            Ok(parsed.response)
        })
    }
}

// ---------------------------------------------------------------------------
// StubProvider — deterministic output for testing / demo mode
// ---------------------------------------------------------------------------

/// Stub LLM provider that returns a deterministic response without
/// any network calls. Used when no Ollama instance is available or
/// during testing.
pub struct StubProvider;

impl LlmProvider for StubProvider {
    fn generate(
        &self,
        prompt: &str,
    ) -> Pin<Box<dyn Future<Output = Result<String, LlmError>> + Send + '_>> {
        // Extract the first line (query) for a readable stub response
        let query = prompt.lines().next().unwrap_or("unknown query").to_string();
        Box::pin(async move {
            Ok(format!(
                "Based on the provided documents, here is the analysis for: {query}"
            ))
        })
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_prompt_matches_hoon_convention() {
        // Must produce: query + \n + chunk0 + \n + chunk1
        let chunks = vec![
            Chunk {
                id: 0,
                dat: "First chunk.".into(),
            },
            Chunk {
                id: 1,
                dat: "Second chunk.".into(),
            },
        ];
        let refs: Vec<&Chunk> = chunks.iter().collect();
        let prompt = build_prompt("What is this?", &refs);

        assert_eq!(prompt, "What is this?\nFirst chunk.\nSecond chunk.");

        // Verify byte-level: separator is exactly 0x0a
        let bytes = prompt.as_bytes();
        let newline_positions: Vec<usize> =
            bytes.iter().enumerate().filter(|&(_, &b)| b == 0x0a).map(|(i, _)| i).collect();
        assert_eq!(newline_positions.len(), 2);
    }

    #[test]
    fn build_prompt_single_chunk() {
        let chunks = vec![Chunk {
            id: 0,
            dat: "Only chunk.".into(),
        }];
        let refs: Vec<&Chunk> = chunks.iter().collect();
        let prompt = build_prompt("Query", &refs);
        assert_eq!(prompt, "Query\nOnly chunk.");
    }

    #[test]
    fn build_prompt_empty_chunks() {
        let refs: Vec<&Chunk> = vec![];
        let prompt = build_prompt("Query", &refs);
        assert_eq!(prompt, "Query");
    }

    #[tokio::test]
    async fn stub_provider_returns_deterministic_output() {
        let provider = StubProvider;
        let result = provider.generate("What is the revenue?\nQ3 data here.").await;
        assert!(result.is_ok());
        let output = result.unwrap();
        assert!(output.contains("What is the revenue?"));
    }

    #[test]
    fn ollama_provider_default_config() {
        let provider = OllamaProvider::default();
        assert_eq!(provider.base_url, "http://localhost:11434");
        assert_eq!(provider.model, "llama3.2");
    }

    #[test]
    fn ollama_provider_custom_config() {
        let provider = OllamaProvider::new("http://gpu-server:11434/", "mistral");
        assert_eq!(provider.base_url, "http://gpu-server:11434");
        assert_eq!(provider.model, "mistral");
    }

    #[test]
    fn llm_error_display() {
        let req_err = LlmError::Request("connection refused".into());
        assert_eq!(
            format!("{req_err}"),
            "LLM request error: connection refused"
        );

        let parse_err = LlmError::Parse("invalid json".into());
        assert_eq!(format!("{parse_err}"), "LLM parse error: invalid json");
    }
}
