.PHONY: build test coverage clean

build:
	cargo build --release -p creditra-credit

test:
	cargo test -p creditra-credit

coverage:
	cargo tarpaulin -p creditra-credit --out html --output-dir coverage --fail-under 95

coverage-xml:
	cargo tarpaulin -p creditra-credit --out xml --output-dir coverage --fail-under 95

clean:
	cargo clean
	rm -rf coverage/
