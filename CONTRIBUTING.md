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
3. If you modified core parsing, config, or TUI logic, run the specific fuzz targets. Use `just fuzz-shell-parse`, `just fuzz-config-cookies`, or `just fuzz-tui-stream` to check for panics.

## Commit Style

We follow [Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/) to automatically generate changelogs and semantic versions via `git-cliff`:

- **One line only**: the subject, no body, no trailers, no blank line
- Ultra-laconic: a few words, 50 chars or fewer, 72 max
- Imperative mood (`fix:` not `fixed:`)
- Conventional prefixes: `feat:`, `fix:`, `docs:`, `refactor:`, `test:`, `chore:`, `ci:`
- Use `!` for breaking changes (e.g., `feat!: new config format`)
- Scopes are encouraged to keep history clean (e.g., `feat(tui): add dark mode`, `fix(shell): handle empty input`)

Put the details and the rationale in the PR description, not in the commit message. `git-cliff` renders only the subject into the changelog.

```
feat(config): add comments support      # good
fix(gemini): kill child process on drop  # good
fix(gemini): prevent cookie leak via process command line  # too long
```

## Code Comments

Add a comment only when the code fails to say it. Do not restate the code, and do not describe the diff. The diff is the record of the change.

Write comments in plain English and ASCII only. No emoji, no em dash, no arrows, no curly quotes. Keep them short and use the active voice.

```rust
// The guard deletes the temp file if the rename fails.   # good
// Loop over the items                                   # bad, the code says it
// Replaced the while let loop with find()               # bad, the diff says it
```

## Releasing

Releases are cut from `main` with `just release`:

```bash
just pre-push      # optional: full compliance gate before releasing
just release       # annotated tag v<version> + push
```

`just release` reads the version from `Cargo.toml`, creates an **annotated** tag and pushes it together with the branch. Annotated tags are required: `git push --follow-tags` silently skips lightweight tags, which is how `v0.3.0` never reached the remote.

If the push step fails (e.g. network), the local tag already exists. Re-run `just release-push` instead of `just release`.

Pushing the tag triggers `.github/workflows/release.yml`, which builds the binaries for all four targets and publishes the GitHub release.

## Security Issues

Please do **NOT** open a public GitHub issue for security vulnerabilities.

See our [Security Policy](../../security) for instructions on how to report issues securely via GitHub's private vulnerability reporting.
