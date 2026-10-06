# Meta Pic Interrogator

[![Release](https://img.shields.io/github/v/release/thejohnd0e/Meta-Pic-Interrogator?label=release)](https://github.com/thejohnd0e/Meta-Pic-Interrogator/releases/latest)
[![Downloads](https://img.shields.io/github/downloads/thejohnd0e/Meta-Pic-Interrogator/total)](https://github.com/thejohnd0e/Meta-Pic-Interrogator/releases)
[![Platform](https://img.shields.io/badge/platform-Windows%2010%20%7C%2011-0078D6?logo=windows&logoColor=white)](https://github.com/thejohnd0e/Meta-Pic-Interrogator/releases/latest)
[![Tauri](https://img.shields.io/badge/Tauri-2-24C8DB?logo=tauri&logoColor=white)](https://tauri.app)
[![Rust](https://img.shields.io/badge/Rust-2021-000000?logo=rust&logoColor=white)](https://www.rust-lang.org)
[![React](https://img.shields.io/badge/React-19-61DAFB?logo=react&logoColor=black)](https://react.dev)
[![License](https://img.shields.io/github/license/thejohnd0e/Meta-Pic-Interrogator)](LICENSE)
[![TypeScript](https://img.shields.io/badge/TypeScript-6-3178C6?logo=typescript&logoColor=white)](https://www.typescriptlang.org)

A small Windows desktop app that describes an image with a vision model and saves a **clean PNG copy** with the description embedded as metadata. Use your **ChatGPT Plus/Pro** or **SuperGrok** subscription, or an OpenAI-compatible API key. The original file is never modified.

## Features

- **Subscription sign-in**: ChatGPT plan usage (Sign in with ChatGPT) and SuperGrok, with no API key needed.
- **API key providers**: OpenAI, OpenAI-compatible endpoints, and Google Gemini (key from AI Studio), with streaming output and a refreshable model list.
- **Editable result**: review and edit the description, choose a preset (Concise, Detailed, Appearance, Clothing, Composition, Photography) or write your own.
- **Metadata that other tools can read**: the saved PNG carries the text in several standard places (see below), so it shows up in viewers such as [Eagle](https://eagle.cool) with a `Parameters` reader plugin.
- **Optional proxy**: HTTP or SOCKS5 for every request the app makes, useful where a provider blocks your region.
- **Light and dark theme**: follows the system or is set in Settings.
- **Remembers your choices**: last provider, model per provider, endpoint, and preset.
- **Private by design**: API keys, refresh tokens, and proxy passwords live in Windows Credential Manager. Access tokens are kept in memory only.
- Input formats: PNG, JPEG, WebP, BMP, TIFF. Drag and drop or use the file dialog.

## Install

Download an installer from the [latest release](https://github.com/thejohnd0e/Meta-Pic-Interrogator/releases/latest):

| File | Scope |
| --- | --- |
| `Meta Pic Interrogator_x.y.z_x64-setup.exe` | Asks whether to install for the current user or for all users |
| `Meta Pic Interrogator_x.y.z_x64_en-US.msi` | All users (needs administrator rights) |

The installers are not code-signed, so Windows SmartScreen may show a warning the first time.

## Saved PNG metadata

Every saved copy is a re-encoded PNG (no source metadata is carried over) with these text chunks:

| Chunk | Content |
| --- | --- |
| `iTXt` `Description` | The description (UTF-8) |
| `iTXt` `Parameters` | The same text, for tools that only read `Parameters` |
| `iTXt` `MetaPic:Interrogator` | Provenance JSON: provider, model, preset, time, app version |
| `iTXt` `XML:com.adobe.xmp` | XMP packet with the description and provenance |

The file is written to a temporary name, parsed back to verify the metadata, and only then renamed into place.

## Notes on providers

- **ChatGPT Plus/Pro** uses OpenAI's Sign in with ChatGPT plan-usage flow for open-source and local apps (OAuth with PKCE and a loopback redirect). This is a preview feature and may change. OpenAI blocks some countries and regions; if sign-in or refresh fails with `unsupported_country_region_territory`, enable the proxy in Settings.
- **SuperGrok** signs in with the OAuth device code flow against `auth.x.ai`, using the shared Grok client id also used by other open-source tools. xAI does not publish third-party documentation for this, and it decides which accounts receive tokens.
- Images are sent only to the provider you select.
- This project is not affiliated with or endorsed by OpenAI or xAI.

## Development

Requirements: Windows 10/11, [Node.js](https://nodejs.org), [Rust](https://rustup.rs) (stable), and the [Tauri prerequisites](https://tauri.app/start/prerequisites/).

```text
npm install
npm run tauri dev
```

Checks:

```text
npm run build
cd src-tauri
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```

Release build:

```text
npm run tauri build
```

The installers are written to `src-tauri/target/release/bundle/`.

### Layout

- `src/`: React frontend (`App.tsx`, typed command wrappers in `src/lib/`).
- `src-tauri/src/`: Rust core: `image/` (decode, normalize, encode), `metadata.rs` (PNG chunks), `providers.rs`, `chatgpt.rs` and `oauth.rs` (ChatGPT), `supergrok.rs`, `network.rs` (proxy), `credentials.rs`, `settings.rs`, `commands.rs`.
- `DESIGN.md`, `DECISIONS.md`: design tokens and recorded decisions.

## License

[MIT](LICENSE)
