.PHONY: build test test-quick lint format check clean index

# Rust (primary)
build:
	cargo build

test:
	cargo test

test-quick:
	cargo test -- --quiet

lint:
	cargo clippy -- -D warnings

format:
	cargo fmt

check: lint test

clean:
	cargo clean
	rm -rf .pruner/ .pytest_cache/ .ruff_cache/
	find . -type d -name __pycache__ -exec rm -rf {} +

index:
	cargo run -- index . -v

# Python (reference implementation)
py-install:
	uv sync

py-test:
	uv run pytest -v

py-lint:
	uv run ruff check src/pruner/ tests/

py-format:
	uv run ruff format src/pruner/ tests/
