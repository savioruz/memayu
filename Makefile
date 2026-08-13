.PHONY: all help build test test-integration lint fmt fmt-check audit check release clean setup dev dev-web

all: check

help: ## Display this help screen
	@awk 'BEGIN {FS = ":.*##"; printf "\nUsage:\n  make \033[36m<target>\033[0m\n"} /^[a-zA-Z_.-]+:.*?##/ { printf "  \033[36m%-15s\033[0m %s\n", $$1, $$2 } /^##@/ { printf "\n\033[1m%s\033[0m\n", substr($$0, 5) } ' $(MAKEFILE_LIST)
.PHONY: help

setup: ## One-time setup (git hooks)
	git config core.hooksPath .githooks

##@ Build

build: ## Build (debug, default features)
	cargo build

check: ## Check-only (fast feedback, all features)
	cargo check --workspace --all-targets --all-features

release: ## Build release binary (all features)
	cargo build --release --all-features

##@ Tests

test: ## Run unit tests (includes embedded-asset checks)
	cargo test --workspace --all-features

test-integration: ## Run integration tests (require live Postgres+pgvector)
	cargo test --all-features

##@ Code quality

lint: ## Lint with clippy (warnings are errors)
	cargo clippy --workspace --all-targets --all-features -- -D warnings

fmt: ## Format code and verify formatting
	cargo fmt --all

fmt-check: ## Check formatting
	cargo fmt --all -- --check

audit: ## Scan dependencies for known vulnerabilities
	cargo audit

##@ Development

dev: ## Run default frontend (TUI) with hot reload
	@scripts/dev.sh tui

dev-web: ## Run web dashboard with hot reload (PORT overridable)
	@scripts/dev.sh web $(if $(PORT),$(PORT),)

clean: ## Clean build artifacts
	cargo clean

