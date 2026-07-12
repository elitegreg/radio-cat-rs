.PHONY: build lint test fmt

build:
	cargo build --all-targets

lint:
	cargo clippy --all-targets --all-features -- -D warnings

test:
	cargo test --all-targets --all-features

fmt:
	cargo fmt --all -- --check
