# MetaPic Interrogator

Windows 10/11 Tauri desktop application for generating image descriptions and saving clean PNG copies with provenance metadata.

## Setup

```text
npm install
```

## Development

```text
npm run tauri dev
```

## Verification

```text
npm run build
cargo fmt --check
cargo test
```

The Rust application is under `src-tauri/`; the React frontend is under `src/`.
See `AGENTS.md` and the approved design in `docs/superpowers/specs/` for project guidance.

## Windows Release

```text
npm run tauri build
```

The bundle identifier is `com.metapic.interrogator`. Signing certificates are intentionally not stored in this repository. xAI, DeepSeek, ChatGPT Plus/Pro, and SuperGrok remain visible but require a successful vision capability probe before image requests are enabled.
