# Contributing to ScreenShotPP

Thanks for your interest in contributing to ScreenShotPP.

ScreenShotPP is a native desktop screenshot tool for macOS and Windows. Contributions that improve reliability, capture behavior, annotation tools, OCR, packaging, documentation, or release readiness are welcome.

## Before you start

For larger changes, please open an issue first so we can discuss the problem, expected behavior, and platform impact.

Small fixes, documentation updates, and targeted improvements can go directly to a pull request.

## Development setup

Prerequisites:

- Rust stable
- macOS: Xcode command line tools
- Windows: Microsoft C++ Build Tools (MSVC + Windows SDK)

Run the app locally:

```bash
cargo run --manifest-path app/Cargo.toml
```

Build locally:

```bash
cargo build --release --manifest-path app/Cargo.toml
```

Video recording needs an `ffmpeg` binary next to the executable
(`scripts/fetch-ffmpeg.sh <windows|macos-arm>` downloads one into `app/binaries/`).

## Checks before opening a pull request

Please run the relevant checks before submitting a change.

Rust tests:

```bash
cargo test --manifest-path app/Cargo.toml --workspace
```

Release readiness check:

```bash
scripts/check-release-readiness.sh
```

If you cannot run a check on your machine, mention that clearly in the pull request.

## Pull request guidelines

- Keep pull requests focused and reasonably small.
- Explain the user-facing change and the reason for it.
- Include screenshots or screen recordings for visible UI changes.
- Mention which platforms you tested: macOS, Windows, or both.
- Update documentation when behavior, setup, shortcuts, or packaging changes.
- Avoid unrelated formatting or refactoring in feature and bug-fix PRs.

## Bug reports

Good bug reports include:

- ScreenShotPP version or commit.
- Operating system and version.
- Steps to reproduce the issue.
- Expected behavior and actual behavior.
- Screenshots, recordings, or logs when useful.

Please do not include sensitive screenshots, private documents, tokens, or personal information in public issues.

## Feature requests

Feature requests are easiest to evaluate when they describe the workflow problem first, then the proposed solution.

Please include the platform impact if the request is specific to macOS, Windows, capture behavior, OCR, shortcuts, packaging, or the annotation editor.

## Code of Conduct

By participating in this project, you agree to follow the [Code of Conduct](CODE_OF_CONDUCT.md).
