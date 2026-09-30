default:
    @just --list

build:
    cargo build --release

fmt:
    cargo fmt

fmt-check:
    cargo fmt --check

clippy:
    cargo clippy --all-targets --all-features -- -D warnings

test:
    cargo test

check: fmt-check clippy test

licenses:
    cargo about generate about.hbs -o THIRD_PARTY_LICENSES.html

clean-licenses:
    rm -f THIRD_PARTY_LICENSES.html PYTHON_LICENSES.txt

python-licenses:
    python3 scripts/collect-python-licenses.py

all-licenses: licenses python-licenses

check-licenses:
    cargo deny check licenses bans sources