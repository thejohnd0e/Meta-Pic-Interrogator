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

The bundle identifier is `com.metapic.interrogator`. Signing certificates are intentionally not stored in this repository. ChatGPT Plus/Pro and SuperGrok are visible in Settings but subscription sign-in is intentionally disabled until an official end-to-end provider flow is implemented.

The current release includes the tested native image/metadata/save pipeline, native open/save dialogs and drag-drop paths, an OpenAI-compatible streaming description path, functional persisted provider settings, preset CRUD, and credential storage commands. It does not yet provide subscription-based inference; official Sign in with ChatGPT plan usage and provider-specific adapters remain in progress. See `STATUS.md` and `TODO.md`.
