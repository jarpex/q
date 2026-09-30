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

gate: check
    cargo audit --deny warnings

licenses:
    cargo about generate about.hbs -o THIRD_PARTY_LICENSES.html

python-licenses:
    python3 scripts/collect-python-licenses.py

all-licenses: licenses python-licenses

clean-licenses:
    rm -f THIRD_PARTY_LICENSES.html PYTHON_LICENSES.txt

check-licenses:
    cargo deny check licenses bans sources

sbom:
    mkdir -p sbom
    cargo cyclonedx --override-filename bom --format json
    mv bom.json sbom/
    ./scripts/clean-sbom.sh sbom/bom.json

clean-sbom:
    rm -rf sbom