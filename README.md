# Codex — DOCaCola fork

A personal fork of [OpenAI Codex](https://github.com/openai/codex), based on
`rust-v0.157.1`. This is an independent build, not an official OpenAI release.
Upstream copyright, Apache-2.0 licensing, and history are preserved.

## Install

Download a Windows x64 ZIP or macOS Apple Silicon tar.gz from
[this fork's releases](https://github.com/DOCaCola/codex/releases) when available.
Extract the **whole archive**, then run `bin/codex.exe` (Windows) or `bin/codex` (macOS).
Keep `bin`, `codex-resources`, `codex-path`, and the license files together.
Packages include the code-mode host, Windows sandbox helpers, ripgrep, and hpatch.
Go is not required. Initial builds are unsigned and omit the optional voice runtime.

This fork targets Windows x64 and macOS Apple Silicon. The inherited npm/Python publishing and
installer scripts target official OpenAI packages; use the fork release archives.
macOS packages have no Developer ID signing or notarization. Desktop integration
and existing database compatibility require validation on a real Mac.

## Fork features

- Windows Git Bash shell integration.
- Experimental compact hpatch edits translated into Codex's native patch flow.
- Command stacks and optional automatic background command completion delivery.
- Optional inbound MCP notifications and account/provider behavior adjustments.

Feature defaults and configuration live in the source. Review
[UPSTREAM_DIVERGENCES.md](UPSTREAM_DIVERGENCES.md) when updating from upstream.

## Build a complete package

On Windows x64, install Git, Python 3.12+, Rust from
`codex-rs/rust-toolchain.toml`, Visual Studio C++ build tools with the Windows SDK,
and Go from [the hpatch source pin](third_party/hpatch/source.json).
Run in a Visual Studio developer environment:

```sh
python scripts/build_fork_release.py --cache-dir C:/build-cache/codex --output-dir dist
```

On an Apple Silicon Mac, install Xcode command-line tools and the same Python,
Rust and pinned Go toolchains, then run:

```sh
python3 scripts/build_fork_release.py --cache-dir "$HOME/Library/Caches/codex-build" --output-dir dist
```

The macOS builder normalizes state migration sources to LF before compilation.
Windows keeps the fork's CRLF migration policy. SQLx hashes these source bytes,
so use the native package builder for the matching platform.

Choose your own cache location. The builder places its temporary files and V8
artifacts on that drive to avoid cross-drive symlink problems. Cargo and Go must be on PATH;
`--cargo` and `--go` accept explicit executables. The build fetches the pinned
hpatch source and matching checksum-verified V8/ripgrep artifacts. It builds all
required Rust companions, produces a platform archive and SHA256SUMS, and bundles licenses
and hpatch source/binary provenance. It never relies on a sibling checkout.

The default `fast-release` profile avoids the cost of ThinLTO; use
`--profile release` for the full optimized build. Local clean builds of the CLI
and code-mode host measured about 7.0 GiB and 9.5 GiB peak target space respectively.
Caches, package assembly, and the rest of the toolchain require additional space.

## GitHub workflow

[Fork release](.github/workflows/fork-release.yml) runs manually or on
`doca-v<workspace-version>` tags, for example `doca-v0.157.1-doca`.
A manual run produces a seven-day artifact. A matching tag also creates a
**draft** GitHub release for review. Later fork versions should use an increasing
`-doca.N` suffix, updating Cargo manifests/lockfiles together.

The workflow uses a Windows runner with four Cargo jobs and a native macOS ARM64
runner with two Cargo jobs, no retained build cache, and no OpenAI signing credentials
or registry publishing. Both packages must pass native CLI and app-server startup
checks before a tag creates a draft containing both archives and combined checksums.
The inherited
OpenAI workflows have been removed from this branch; their source remains in the
upstream history. Do not re-enable them wholesale when merging upstream.

## Maintenance and attribution

Use `origin` for this fork and `upstream` for `https://github.com/openai/codex.git`.
Merge reviewed upstream releases into the maintained fork branch. Preserve
upstream authors; author fork changes as your GitHub identity. The hpatch companion
is maintained separately in [DOCaCola/hpatch](https://github.com/DOCaCola/hpatch);
see [its integration/build contract](third_party/hpatch/README.md).

Codex: [Apache-2.0](LICENSE), [NOTICE](NOTICE). hpatch: [MIT](third_party/hpatch/LICENSE).
