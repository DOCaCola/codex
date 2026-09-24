# Fork workflows

`fork-release.yml` is the maintained Windows x64 build. Run it manually for a
short-lived artifact, or push a `doca-v<workspace-version>` tag to create a draft
release. See the root README for prerequisites, packaging, and limitations.

The upstream OpenAI workflows are intentionally absent here. They assume private
runner groups, signing credentials, hosted caches, and official registry access.
Their originals remain available in the `rust-v0.156.1` history. Review upstream
workflow additions before enabling them on this fork.
