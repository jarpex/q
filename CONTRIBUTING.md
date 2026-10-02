# Contributing to q

Thanks for your interest! This is a solo-maintained project.

## Requirements

Install [just](https://github.com/casey/just) as your command runner:

```bash
brew install just  # or your OS equivalent
```

Install Rust development tools:

```bash
cargo install cargo-audit cargo-deny cargo-about cargo-fuzz cargo-cyclonedx
rustup component add clippy rustfmt
```

Install external scanners:

```bash
brew install gitleaks trivy vexctl
```

## Workflow

Run `just` to list all available commands. Here are the most common workflows:

```bash
cargo test              # Run all unit and integration tests
just check              # Quick check: fmt, clippy, and tests
just scan               # Security gate: check + audit, trivy, gitleaks, sast
just compliance         # scan + license/ban checks + regenerate THIRD_PARTY_LICENSES.html
just pre-push           # The ultimate combo: runs everything before pushing to remote
just fuzz-all           # Run all fuzzing targets (shell, config, tui)
```

## Before Opening a PR

Open an issue for large changes first. For the PR to be accepted, CI runs strict security and compliance gates. Please verify your changes locally beforehand:

1. `cargo test` passes
2. `just pre-push` passes (this ensures formatting, lints, tests, security scans, and license checks are all green)
3. If you modified core parsing, config, or TUI logic, run the specific fuzz targets (e.g., `just fuzz-shell-parse`, `just fuzz-config-cookies`, or `just fuzz-tui-stream`) to ensure no panics occur.

## Commit Style

We follow [Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/) to automatically generate changelogs and semantic versions via `git-cliff`:

- Imperative mood (`fix:` not `fixed:`)
- Conventional prefixes: `feat:`, `fix:`, `docs:`, `refactor:`, `test:`, `chore:`, `ci:`
- Use `!` for breaking changes (e.g., `feat!: new config format`)
- Scopes are encouraged to keep history clean (e.g., `feat(tui): add dark mode`, `fix(shell): handle empty input`)

## Security Issues

Please do **NOT** open a public GitHub issue for security vulnerabilities.

See our [Security Policy](../../security) for instructions on how to report issues securely via GitHub's private vulnerability reporting.
