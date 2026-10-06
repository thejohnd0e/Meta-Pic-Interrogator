# Decisions

Newest decisions are at the end. A decision marked Superseded is kept for context only.

## 2026-10-05: Choose Tauri, React, Rust

- Decision: Build Meta Pic Interrogator with Tauri 2, React 19/TypeScript, and Rust for Windows 10/11.
- Reason: This keeps image and metadata handling in a typed native core and follows the provider and credential patterns of the author's LingvoLoc project.
- Status: Active. (Replaces the initial "Mark Unknown Project Details as TBD" decision.)

## 2026-10-05: Save Clean PNG Copies

- Decision: Never modify the source; always save a new clean PNG containing the description and provenance metadata. `save_png_copy` decodes the source, re-encodes a clean PNG, writes iTXt/XMP metadata, reparses it, and renames atomically only after verification.
- Reason: Prevents accidental source loss and gives the metadata contract a deterministic, verifiable output.
- Status: Active.

## 2026-10-05: Gate Unsupported Vision Providers

- Decision: Keep the full provider list in the product model, but enable image requests only for providers with a working vision adapter. Unavailable providers are shown disabled and cannot be selected (Anthropic and OpenRouter at the time of writing).
- Reason: Never send image bytes to a provider whose contract is unverified.
- Status: Active.

## 2026-10-05: Subscription-First Authentication

- Decision: Prefer official subscription sign-in over API keys, which remain a fallback. Do not use browser cookies, web scraping, or undocumented endpoints.
- Reason: Subscription UI alone is not authentication, and scraped sessions are fragile and unsafe.
- Status: Active. The clause "keep SuperGrok disabled" is superseded by the 2026-10-06 SuperGrok decision; Gemini subscriptions are covered by the 2026-10-06 Gemini decision.

## 2026-10-05: Use Dynamic Registration For ChatGPT Plan Usage

- Decision: Use OpenAI's documented open-source "plan usage" flow: first sign-in with `client_id=dynamic_agent_client`, persist the issued client id, a stable `ext_agent_host_id`, and the refresh token; PKCE S256; loopback redirect `http://127.0.0.1:47836/auth/callback`; scope `openid profile email offline_access resource.invoke chatgpt.tokens.use.direct`; resource `https://api.openai.com/v1`; ID token validated against the JWKS. Inference uses the Responses API with `store:false` and `stream:true`. Do not use another product's client id.
- Reason: OpenAI documents this flow for open-source and local apps. It is a preview and may change.
- Confirmed on the live service: dynamic registration, the callback `client_id` parameter, the `instructions` field, model list, and image descriptions. Not confirmed: the revoke form fields.
- Status: Active.

## 2026-10-06: Credential Storage

- Decision: API keys, the proxy password, and OAuth session records (client id, host id, e-mail, refresh token) live in Windows Credential Manager under the service `metapic-interrogator`. Access tokens stay in memory only. Non-secret settings live in `settings.json` in the app data directory.
- Reason: Keeps secrets out of files and logs.
- Status: Active. The service name is not renamed so existing stored credentials keep working.

## 2026-10-06: Mirror Description Into Parameters

- Decision: Saved PNGs carry the description as iTXt `Description` and as iTXt `Parameters`, plus `MetaPic:Interrogator` provenance JSON and XMP. The verified save requires `Parameters` to equal `Description`.
- Reason: The Eagle "PNG metadata" plugin reads only `Parameters` (Stable Diffusion style). Tools that parse `Parameters` as an SD prompt will show the description as the prompt text.
- Status: Active. The `MetaPic:Interrogator` chunk name and the XMP namespace are kept for compatibility with saved files.

## 2026-10-06: Support SuperGrok Subscription Sign-In

- Decision: Sign in with the OAuth device code flow (RFC 8628) against auth.x.ai using the shared Grok client id, then call the Responses API on api.x.ai/v1. SuperGrok is a normal supported provider.
- Reason: The user confirmed this works for their subscription in other apps. xAI publishes no third-party OAuth documentation, so the client id and endpoints come from open-source implementations and the LingvoLoc module; xAI decides which accounts get tokens and may change this.
- Status: Active; confirmed working on Windows.

## 2026-10-06: Gemini Through An API Key, Not A Subscription

- Decision: Support Google Gemini with an AI Studio API key (REST `streamGenerateContent`, key sent in the `x-goog-api-key` header). Do not add sign-in for Google AI Pro/Plus subscriptions.
- Reason: Google offers no OAuth scope for third-party use of a consumer subscription. The Gemini CLI OAuth route was deprecated for consumer tiers on 2026-06-18 and Google calls its use in third-party software a policy violation that can lead to account suspension.
- Status: Active; confirmed working with a key.

## 2026-10-06: Optional Proxy For All Outbound Requests

- Decision: One optional HTTP or SOCKS5 proxy (`network.rs`), configured in Settings, applied through a shared client builder to every outbound client (sign-in, Responses, API-key providers, model lists). SOCKS5 uses `socks5h` so the proxy resolves DNS. The sign-in page opened in the browser does not use it.
- Reason: OpenAI blocks token refresh and API calls from unsupported regions (`403 unsupported_country_region_territory`).
- Status: Active.

## 2026-10-06: Per-Provider Model Memory And Model Lists

- Decision: `settings.json` stores the last provider and a model per provider (`modelByProvider`), saved automatically with a short debounce. Switching providers restores that provider's model or a default, and never carries a model over. Model lists are fetched from each provider (subscription and API-key) with a refresh control; unavailable providers cannot be selected.
- Reason: A model id from one provider is invalid for another.
- Status: Active.

## 2026-10-06: Keep The UI Thread Free

- Decision: Commands that do file IO, decoding, or network work are `async` and run through `off_ui_thread` (`spawn_blocking`).
- Reason: Synchronous Tauri commands run on the window's main thread; a provider request froze the window.
- Status: Active. New long-running commands must follow this rule.

## 2026-10-06: Layout And Theme

- Decision: The main screen fits the window height without page scrolling (`.app-shell.fit` in `App.css`) and falls back to normal scrolling below 680px height or 760px width; Settings scrolls normally. Colors are CSS variables on `:root` with a `data-theme="light|dark"` override; the choice (system, light, dark) is stored in `localStorage` and applied before first paint.
- Reason: Avoid scrolling in the working screen; keep one place for color tokens.
- Status: Active.

## 2026-10-06: Product Name And Identifiers

- Decision: The display name is "Meta Pic Interrogator". The bundle identifier `com.metapic.interrogator`, the crate and package name, and the credential service name are unchanged.
- Reason: Renaming identifiers would break upgrades and stored credentials. Changing the product name makes installers create a separate install folder, so older "MetaPic Interrogator" installs must be removed manually.
- Status: Active.

## 2026-10-06: Installers

- Decision: Ship an NSIS installer with `installMode: both` (current user or all users) and an MSI (per-machine only; Tauri's WiX has no per-user mode). Installers are not code-signed. Bump the version for every build handed to testers so upgrades apply.
- Status: Active.

## 2026-10-06: Presets Are Markdown Files

- Decision: Each preset is one file, `<app data>/presets/<id>.md`, with a front matter `name:` line followed by the prompt. The id is the file name, derived from the name (letters, digits, `-`, `_`; Windows reserved names avoided) and never chosen by the UI. Presets that older versions kept in `settings.json` are moved into files once, when the folder does not exist yet; the folder's existence is the migration marker, so deleting every preset does not restore the defaults. Files without front matter are read as plain prompts named after the file. Limits: 1 MB per prompt, 200 files per import. The list is alphabetical.
- Reason: Prompts can be long Markdown documents and numerous; files can be edited in any editor, versioned, and copied between machines, and keep `settings.json` small.
- Status: Active. Editing is plain text; the editor autosaves after 0.7 s and on leaving.

## 2026-10-06: Keep Agent Files Out Of The Public Repository

- Decision: `AGENTS.md`, `CLAUDE.md`, `docs/superpowers/`, `STATUS.md`, `TODO.md`, and agent tool directories are listed in `.gitignore` and were removed from all git history. Local copies remain for coding agents.
- Reason: They are working notes, not project documentation. The repository is public (MIT).
- Status: Active. `DECISIONS.md`, `DESIGN.md`, and `README.md` are the published documents.
