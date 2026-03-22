.PHONY: build test test-quick lint format check clean index

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
	rm -rf .pruner/

index:
	cargo run -- index . -v
