.PHONY: build release run test test-quick test-unit test-integration bench lint format check clean index

build:
	cargo build

release:
	cargo build --release

test:
	cargo test --bin pruner --test integration

test-unit:
	cargo test --lib

test-integration:
	cargo test --test integration

test-quick:
	cargo test -- --quiet

bench:
	cargo build --release
	cargo test --test bench -- --nocapture

lint:
	cargo clippy -- -D warnings

format:
	cargo fmt

check: lint test

clean:
	cargo clean
	rm -rf .pruner/

run:
	cargo run -- $(ARGS)

index:
	cargo run -- index . -v
