VERSION := `grep -E '^version\s*=' Cargo.toml | head -n 1 | sed 's/.*"\(.*\)".*/\1/'`

default:
    @just --list

fmt:
    cargo fmt

fmt-check:
    cargo fmt --check

clippy:
    cargo clippy --all-targets --all-features -- -D warnings

test:
    cargo test --all-features

build:
    cargo build --release

check: fmt-check clippy test

clean: clean-sbom clean-licenses
    cargo clean

licenses:
    cargo about generate about.hbs -o THIRD_PARTY_LICENSES.html

python-licenses:
    python3 scripts/collect-python-licenses.py

all-licenses: licenses python-licenses

clean-licenses:
    rm -f THIRD_PARTY_LICENSES.html PYTHON_LICENSES.txt

check-licenses:
    cargo deny check licenses bans sources

_mkdir-sbom:
    @mkdir -p sbom

sbom: _mkdir-sbom
    cargo cyclonedx --override-filename bom --format json
    mv bom.json sbom/
    ./scripts/clean-sbom.sh sbom/bom.json

clean-sbom:
    rm -rf sbom

secrets: _mkdir-sbom
    gitleaks detect --config .gitleaks.toml --source . --report-format sarif --report-path sbom/gitleaks.sarif --no-git --log-level warn

sast: _mkdir-sbom
    ./scripts/clippy-sarif.sh sbom/clippy.sarif

audit-gate:
    cargo audit --deny warnings --file .cargo/audit.toml

audit-sarif: _mkdir-sbom
    cargo audit --format sarif --file .cargo/audit.toml > sbom/cargo-audit.sarif

sca-general: sbom
    ./scripts/trivy-scan.sh sbom/bom.json sbom/trivy-vuln.sarif

scan: check audit-gate secrets sast sca-general

compliance: scan check-licenses all-licenses

pre-push: compliance

vex-create vuln subcomponent justification statement:
    mkdir -p vex/statements
    vexctl create \
      --product "pkg:cargo/q@{{VERSION}}" \
      --subcomponents "pkg:cargo/{{subcomponent}}" \
      --vuln "{{vuln}}" \
      --status "not_affected" \
      --justification "{{justification}}" \
      --impact-statement "{{statement}}" \
      --file "vex/statements/q-{{vuln}}.vex.json"
    @just vex-merge

vex-merge:
    vexctl merge vex/statements/*.vex.json > vex/q.vex.json

vex-list:
    @ls -1 vex/statements/*.vex.json 2>/dev/null || echo "No VEX documents found"

fuzz-build:
    cargo fuzz build

_fuzz-target target time="60":
    cargo fuzz run {{target}} -- -max_total_time={{time}}

fuzz-shell-extract time="60":
    just _fuzz-target shell_extract {{time}}

fuzz-shell-parse time="60":
    just _fuzz-target shell_parse {{time}}

fuzz-config-cookies time="60":
    just _fuzz-target config_cookies {{time}}

fuzz-config-metadata time="60":
    just _fuzz-target config_metadata {{time}}

fuzz-tui-wrap time="60":
    just _fuzz-target tui_wrap {{time}}

fuzz-tui-stream time="60":
    just _fuzz-target tui_stream {{time}}

fuzz-parse: fuzz-shell-parse

fuzz-all time="60":
    just fuzz-shell-extract {{time}}
    just fuzz-shell-parse {{time}}
    just fuzz-config-cookies {{time}}
    just fuzz-config-metadata {{time}}
    just fuzz-tui-wrap {{time}}
    just fuzz-tui-stream {{time}}
