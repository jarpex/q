VERSION := `grep -E '^version\s*=' Cargo.toml | head -n 1 | sed 's/.*"\(.*\)".*/\1/'`

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

secrets:
    mkdir -p sbom
    gitleaks detect --source . --report-format sarif --report-path sbom/gitleaks.sarif --no-git --log-level warn

sast:
    mkdir -p sbom
    ./scripts/clippy-sarif.sh sbom/clippy.sarif

audit-gate:
    cargo audit --deny warnings --file audit.toml

audit-sarif:
    mkdir -p sbom
    cargo audit --format sarif --file audit.toml > sbom/cargo-audit.sarif

sca-general: sbom
    ./scripts/trivy-scan.sh sbom/bom.json sbom/trivy-vuln.sarif

scan: fmt-check audit-gate audit-sarif secrets sast sca-general

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

fuzz-shell-extract time="60":
    cargo fuzz run shell_extract -- -max_total_time={{time}}

fuzz-shell-parse time="60":
    cargo fuzz run shell_parse -- -max_total_time={{time}}

fuzz-config-cookies time="60":
    cargo fuzz run config_cookies -- -max_total_time={{time}}

fuzz-config-metadata time="60":
    cargo fuzz run config_metadata -- -max_total_time={{time}}

fuzz-tui-wrap time="60":
    cargo fuzz run tui_wrap -- -max_total_time={{time}}

fuzz-tui-stream time="60":
    cargo fuzz run tui_stream -- -max_total_time={{time}}

fuzz-all time="60":
    cargo fuzz run shell_extract -- -max_total_time={{time}}
    cargo fuzz run shell_parse -- -max_total_time={{time}}
    cargo fuzz run config_cookies -- -max_total_time={{time}}
    cargo fuzz run config_metadata -- -max_total_time={{time}}
    cargo fuzz run tui_wrap -- -max_total_time={{time}}
    cargo fuzz run tui_stream -- -max_total_time={{time}}