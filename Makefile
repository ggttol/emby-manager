.PHONY: rust-check rust-test web-install web-build web-test compose-up compare-rs-py

rust-check:
	cargo check

rust-test:
	cargo test

web-install:
	cd web && npm install

web-build:
	cd web && npm run build

web-test:
	cd web && npm test

compose-up:
	docker compose up --build

compare-rs-py:
	python3 scripts/compare_rust_python.py

