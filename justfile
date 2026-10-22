set windows-shell := ["powershell.exe", "-NoLogo", "-Command"]

VERSION := if os_family() == "windows" {
    `py -c "import tomllib; print(tomllib.load(open('Cargo.toml', 'rb'))['package']['version'])"`
} else {
    `grep -E '^version\s*=' Cargo.toml | head -n 1 | sed 's/.*"\(.*\)".*/\1/'`
}

PYTHON_LAUNCHER := if os_family() == "windows" { "py" } else { "python3" }

# Current branch, resolved lazily when a recipe uses it.
BRANCH := `git rev-parse --abbrev-ref HEAD`

CLEAN_SBOM := if os_family() == "windows" { "powershell.exe -NoProfile -ExecutionPolicy Bypass -File scripts/clean-sbom.ps1" } else { "./scripts/clean-sbom.sh" }
CLIPPY_SARIF := if os_family() == "windows" { "powershell.exe -NoProfile -ExecutionPolicy Bypass -File scripts/clippy-sarif.ps1" } else { "./scripts/clippy-sarif.sh" }
TRIVY_SCAN := if os_family() == "windows" { "powershell.exe -NoProfile -ExecutionPolicy Bypass -File scripts/trivy-scan.ps1" } else { "./scripts/trivy-scan.sh" }
MOVE_BOM := if os_family() == "windows" { "Move-Item -Force bom.json sbom/bom.json" } else { "mv bom.json sbom/" }

_mkdir-sbom:
    {{if os_family() == "windows" { "New-Item -ItemType Directory -Force -Path sbom | Out-Null" } else { "mkdir -p sbom" }}}

default:
    @just --list

fmt:
    cargo +nightly fmt

fmt-check:
    cargo +nightly fmt --check

clippy:
    cargo clippy --all-targets --all-features -- -D warnings

test:
    cargo test --all-features

# Line coverage. Needs the nightly llvm-tools-preview component.
coverage:
    cargo llvm-cov --all-features --summary-only

build:
    cargo build --release

check: fmt-check clippy test

clean: clean-sbom clean-licenses
    cargo clean

licenses:
    cargo about generate about.hbs -o THIRD_PARTY_LICENSES.html

python-licenses:
    {{PYTHON_LAUNCHER}} scripts/collect-python-licenses.py

all-licenses: licenses python-licenses

clean-licenses:
    rm -f THIRD_PARTY_LICENSES.html PYTHON_LICENSES.txt

check-licenses:
    cargo deny check licenses bans sources

sbom: _mkdir-sbom
    cargo cyclonedx --override-filename bom --format json
    {{MOVE_BOM}}
    {{CLEAN_SBOM}} sbom/bom.json

clean-sbom:
    rm -rf sbom

secrets: _mkdir-sbom
    gitleaks detect --config .gitleaks.toml --source . --report-format sarif --report-path sbom/gitleaks.sarif --no-git --log-level warn

sast: _mkdir-sbom
    {{CLIPPY_SARIF}} sbom/clippy.sarif

audit-gate:
    cargo audit --deny warnings --file .cargo/audit.toml

audit-sarif: _mkdir-sbom
    cargo audit --format sarif --file .cargo/audit.toml > sbom/cargo-audit.sarif

sca-general: sbom
    {{TRIVY_SCAN}} sbom/bom.json sbom/trivy-vuln.sarif

scan: check audit-gate secrets sast sca-general

compliance: scan check-licenses all-licenses

pre-push: compliance

# Refuse to release from a detached HEAD: there is no branch to push.
_require-branch:
    @git symbolic-ref --quiet HEAD >/dev/null

# The tag must be annotated: `git push --follow-tags` skips lightweight tags.
release: _require-branch
    scripts/check-release-tag.sh "v{{VERSION}}"
    @git tag -a "v{{VERSION}}" -m "Release v{{VERSION}}"
    @git push origin "refs/heads/{{BRANCH}}:refs/heads/{{BRANCH}}" "refs/tags/v{{VERSION}}"

# Push a tag that `just release` already created.
release-push:
    @git push origin "refs/heads/{{BRANCH}}:refs/heads/{{BRANCH}}" "refs/tags/v{{VERSION}}"

# Install the man page into the local man directory.
install-man:
    #!/usr/bin/env bash
    set -euo pipefail
    MAN_DIR="${HOME}/.local/share/man/man1"
    mkdir -p "${MAN_DIR}"
    cp man/q.1 "${MAN_DIR}/"
    echo "Installed man page to ${MAN_DIR}/q.1"

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
    cargo +nightly fuzz build

_fuzz-target target time="60":
    cargo +nightly fuzz run {{target}} -- -max_total_time={{time}}

fuzz-shell-extract time="60":
    just _fuzz-target shell_extract {{time}}

fuzz-shell-parse time="60":
    just _fuzz-target shell_parse {{time}}

fuzz-config-cookies time="60":
    just _fuzz-target config_cookies {{time}}

fuzz-config-metadata time="60":
    just _fuzz-target config_metadata {{time}}

fuzz-config-settings time="60":
    just _fuzz-target config_settings {{time}}

fuzz-config-auth-timeout time="60":
    just _fuzz-target config_auth_timeout {{time}}

fuzz-tui-wrap time="60":
    just _fuzz-target tui_wrap {{time}}

fuzz-tui-stream time="60":
    just _fuzz-target tui_stream {{time}}

fuzz-uc-args time="60":
    just _fuzz-target uc_args {{time}}

fuzz-parse: fuzz-shell-parse

fuzz-all time="60":
    just fuzz-shell-extract {{time}}
    just fuzz-shell-parse {{time}}
    just fuzz-config-cookies {{time}}
    just fuzz-config-metadata {{time}}
    just fuzz-config-settings {{time}}
    just fuzz-config-auth-timeout {{time}}
    just fuzz-tui-wrap {{time}}
    just fuzz-tui-stream {{time}}
    just fuzz-uc-args {{time}}
