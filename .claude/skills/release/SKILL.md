---
name: release
description: "Release a new version: bump version, update docs, commit, push, tag, and verify the release workflow"
argument-hint: "<major|minor|patch>"
---

Release a new version of ailloy.

## Input

$ARGUMENTS must be one of: `major`, `minor`, `patch`. If empty or invalid, stop and ask.

## Steps

### 1. Determine the new version

- Read the current version from the `version` field in `Cargo.toml`
- Apply the semver bump based on $ARGUMENTS:
  - `patch`: 0.1.0 -> 0.1.1
  - `minor`: 0.1.0 -> 0.2.0
  - `major`: 0.1.0 -> 1.0.0
- Show the user: "Releasing ailloy v{OLD} -> v{NEW}"

### 2. Update toolchain and dependencies

- Run `rustup update stable` — CI runs the LATEST stable Rust, and newer clippy versions ship new
  lints. Running the pre-flight checks on an older local toolchain lets warnings through that then
  fail the release workflow. After updating, confirm with `rustc --version`
- Run `cargo update` to update all dependencies to the latest compatible versions

### 3. Pre-flight checks

- Run `cargo fmt --all -- --check` — abort if formatting issues. If you fix formatting with
  `cargo fmt --all`, re-run clippy afterwards: reformatting can change what clippy flags
- Run `cargo clippy --all-targets -- -D warnings` — abort if warnings (matches CI: it also lints
  tests and benches)
- Run `cargo test` — abort if any test fails
- Run `git status` — abort if there are uncommitted changes that are NOT documentation, version,
  or dependency files (note: `Cargo.lock` changes from `cargo update` are expected)

### 4. Bump version number

- Update `version` in `Cargo.toml`

### 5. Update documentation

- **CHANGELOG.md**: Rename the `[Unreleased]` section to `[{NEW_VERSION}] - {TODAY}` (YYYY-MM-DD format). If there is no `[Unreleased]` section, create a new dated entry summarizing changes since the last release
- **README.md**: Review for accuracy — update any version references if present
- **CLAUDE.md**: Review for accuracy — no version references to update typically
- **INSTALL.md**: Review for accuracy — no version references to update typically

### 6. Verify the build

- Run `cargo build` to ensure everything compiles with the new version
- Run `cargo test` once more after version bump

### 7. Commit, push, and tag

- Stage all changed files: `Cargo.toml`, `Cargo.lock`, `CHANGELOG.md`, and any updated docs
- Commit with message: `Release v{NEW_VERSION}`
- Push to main: `git push`
- Create and push tag: `git tag v{NEW_VERSION} && git push origin v{NEW_VERSION}`

### 8. Watch and verify

- The tag push triggers the Release workflow. Do NOT declare success yet — watch it:
  `gh run list --repo mklab-se/ailloy --workflow release.yml --limit 1`, then
  `gh run watch <id> --repo mklab-se/ailloy --exit-status` until it completes
- If it fails, inspect with `gh run view <id> --log-failed`, fix the cause, and re-release as a patch
- When it is green, confirm the outputs:
  - `gh release view v{NEW_VERSION} --repo mklab-se/ailloy` lists 4 archives
    (3 × `.tar.gz`, 1 × `.zip`) plus 4 matching `.cdx.json` SBOMs
  - `cargo search ailloy --limit 1` shows the new version on crates.io
  - `Formula/ailloy.rb` in `mklab-se/homebrew-tap` carries the new version

### 9. Confirm

- Tell the user the release is tagged, pushed, and the workflow is green — auditable binaries and
  SBOMs are attached to the GitHub Release, crates.io is published, and the Homebrew tap is updated
- The publish jobs require the `CARGO_REGISTRY_TOKEN` (in the `crates-io` environment) and
  `HOMEBREW_TAP_TOKEN` (repo secret) to be configured — see README.md
