# Vesl — Makefile
#
# Quick start:
#   cp vesl.toml.example vesl.toml   (edit nock_home if needed)
#   make setup                        (create hoon symlinks)
#   make build                        (compile hull)
#   make demo-local                   (run the pipeline, no chain)

.PHONY: help setup build test test-unit demo demo-local kernel clean status

# ---------------------------------------------------------------------------
# Config: vesl.toml → env var fallback → empty
# ---------------------------------------------------------------------------

NOCK_HOME ?= $(shell grep -s '^nock_home' vesl.toml 2>/dev/null | sed 's/.*= *"\(.*\)"/\1/' | head -1)
OLLAMA_URL ?= $(shell grep -s '^ollama_url' vesl.toml 2>/dev/null | sed 's/.*= *"\(.*\)"/\1/' | head -1)
API_PORT ?= $(shell grep -s '^api_port' vesl.toml 2>/dev/null | sed 's/.*= *\([0-9]*\)/\1/' | head -1)

# ---------------------------------------------------------------------------
# Default target
# ---------------------------------------------------------------------------

help:
	@echo "Vesl — verified RAG on Nockchain"
	@echo ""
	@echo "Quick start:"
	@echo "  cp vesl.toml.example vesl.toml   # edit nock_home if needed"
	@echo "  make setup                        # create hoon symlinks"
	@echo "  make build                        # compile hull"
	@echo "  make demo-local                   # run the pipeline"
	@echo ""
	@echo "Targets:"
	@echo "  setup       Create hoon/ symlinks to nockchain monorepo"
	@echo "  build       Compile hull (cargo build --release)"
	@echo "  test        Run all tests (unit + e2e)"
	@echo "  test-unit   Run unit tests only"
	@echo "  demo        Full demo with fakenet (requires nockchain in PATH)"
	@echo "  demo-local  Local-only demo (no chain, stub LLM unless ollama configured)"
	@echo "  kernel      Recompile Hoon kernel to assets/vesl.jam"
	@echo "  clean       Remove build artifacts and runtime state"
	@echo "  status      Show fakenet status"
	@echo ""
	@echo "Config: set values in vesl.toml or via environment variables."
	@echo "  NOCK_HOME   = $(or $(NOCK_HOME),(not set))"
	@echo "  OLLAMA_URL  = $(or $(OLLAMA_URL),(not set))"
	@echo "  API_PORT    = $(or $(API_PORT),(not set))"

# ---------------------------------------------------------------------------
# Prerequisite checks
# ---------------------------------------------------------------------------

check-cargo:
	@command -v cargo >/dev/null 2>&1 || { \
		echo "Error: cargo not found."; \
		echo "Install Rust: https://rustup.rs"; \
		echo "Required nightly: $$(cat hull/rust-toolchain 2>/dev/null || echo 'see hull/rust-toolchain')"; \
		exit 1; \
	}

check-nock-home:
	@if [ -z "$(NOCK_HOME)" ]; then \
		echo "Error: NOCK_HOME is not set."; \
		echo ""; \
		echo "Option 1: Create vesl.toml from the template:"; \
		echo "  cp vesl.toml.example vesl.toml"; \
		echo ""; \
		echo "Option 2: Set the environment variable:"; \
		echo "  export NOCK_HOME=~/projects/nockchain/nockchain"; \
		exit 1; \
	fi
	@if [ ! -d "$(NOCK_HOME)/hoon/common" ]; then \
		echo "Error: $(NOCK_HOME)/hoon/common not found."; \
		echo "Is NOCK_HOME pointing to the nockchain monorepo root?"; \
		echo "  Current value: $(NOCK_HOME)"; \
		exit 1; \
	fi

check-hoonc:
	@command -v hoonc >/dev/null 2>&1 || { \
		echo "Error: hoonc not found."; \
		echo "Build it from the nockchain monorepo:"; \
		echo "  cd $(or $(NOCK_HOME),\$$NOCK_HOME) && make install-hoonc"; \
		exit 1; \
	}

check-nockchain:
	@command -v nockchain >/dev/null 2>&1 || { \
		echo "Error: nockchain binary not found."; \
		echo "Build it from the nockchain monorepo:"; \
		echo "  cd $(or $(NOCK_HOME),\$$NOCK_HOME) && make install-nockchain"; \
		exit 1; \
	}

# ---------------------------------------------------------------------------
# Targets
# ---------------------------------------------------------------------------

setup: check-cargo check-nock-home
	@NOCK_HOME="$(NOCK_HOME)" ./scripts/setup-hoon-tree.sh

build: check-cargo
	cd hull && cargo build --release

test: check-cargo
	cd hull && cargo test

test-unit: check-cargo
	cd hull && cargo test --lib

demo: check-cargo check-nockchain
	@DEMO_FLAGS=""; \
	if [ -n "$(OLLAMA_URL)" ]; then DEMO_FLAGS="$$DEMO_FLAGS --ollama-url $(OLLAMA_URL)"; fi; \
	if [ -n "$(API_PORT)" ]; then DEMO_FLAGS="$$DEMO_FLAGS --port $(API_PORT)"; fi; \
	./scripts/demo.sh $$DEMO_FLAGS

demo-local: check-cargo
	@DEMO_FLAGS="--no-chain"; \
	if [ -n "$(OLLAMA_URL)" ]; then DEMO_FLAGS="$$DEMO_FLAGS --ollama-url $(OLLAMA_URL)"; fi; \
	if [ -n "$(API_PORT)" ]; then DEMO_FLAGS="$$DEMO_FLAGS --port $(API_PORT)"; fi; \
	./scripts/demo.sh $$DEMO_FLAGS

kernel: check-cargo check-nock-home check-hoonc
	hoonc --new protocol/lib/vesl-kernel.hoon hoon/
	cp out.jam assets/vesl.jam
	rm -f out.jam
	@echo "Kernel compiled -> assets/vesl.jam"

clean:
	@if [ -x scripts/fakenet-harness.sh ]; then ./scripts/fakenet-harness.sh stop 2>/dev/null || true; fi
	cd hull && cargo clean 2>/dev/null || true
	rm -rf .fakenet/ hull/.data.vesl/ out.jam
	@echo "Clean."

status:
	@if [ -x scripts/fakenet-harness.sh ]; then \
		./scripts/fakenet-harness.sh status; \
	else \
		echo "No fakenet harness found."; \
	fi
