# Decisions

## 2026-10-05: Initialize Shared Agent Documentation

- Decision: Use `AGENTS.md` as the canonical instructions file and keep `CLAUDE.md` as a thin pointer.
- Reason: This supports Codex, Claude Code, and OpenCode without duplicating shared guidance.
- Status: Active.

## 2026-10-05: Mark Unknown Project Details as TBD

- Decision: Do not choose a runtime, framework, dependency manager, or commands before project requirements are known.
- Reason: The workspace was empty during initialization.
- Status: Active.

## 2026-10-05: Choose Tauri React Rust

- Decision: Build MetaPic Interrogator with Tauri 2, React 19/TypeScript, and Rust.
- Reason: This matches LingvoLoc's provider and credential patterns and PromptLens's Windows desktop foundation while keeping image and metadata handling in a typed native core.
- Status: Active.

## 2026-10-05: Save Clean PNG Copies

- Decision: Never modify the source and always save a new clean PNG containing Description and provenance metadata.
- Reason: This prevents accidental source loss and gives the metadata contract a deterministic output format.
- Status: Active.

## 2026-10-05: Gate Unsupported Vision Providers

- Decision: Keep the full requested cloud provider list in the product model, but enable image requests only after a provider/model vision capability is verified.
- Reason: LingvoLoc's subscription, xAI, and DeepSeek paths are currently text-oriented; a capability gate avoids falsely advertising unsupported image input without blocking the app release.
- Status: Active.

## 2026-10-05: Keep Provider Commands Capability-Gated

- Decision: Ship normalized provider payload builders and mock transport tests before enabling live image requests; unsupported or unprobed providers remain unavailable.
- Reason: The current implementation has no verified credentials or live-provider contract evidence, and must not send image bytes optimistically.
- Status: Active.

## 2026-10-05: Preserve Native Save Path

- Decision: `save_png_copy` decodes the source, re-encodes clean PNG, writes iTXt/XMP provenance, reparses metadata, and atomically renames only after verification.
- Reason: This enforces source immutability and prevents publishing an unverifiable output file.
- Status: Active.

## 2026-10-05: Subscription-First Authentication

- Decision: Treat official subscription OAuth as the primary product path; API keys remain a secondary fallback. Do not use browser cookies, web scraping, or undocumented subscription endpoints.
- Reason: ChatGPT plan usage has an official Sign in with ChatGPT flow, while subscription UI alone is not authentication. SuperGrok must remain disabled until xAI provides a verified third-party inference OAuth surface.
- Status: Active; ChatGPT OAuth/Responses adapter is the next implementation slice.

## 2026-10-05: Use Dynamic Registration For ChatGPT Plan Usage

- Decision: Use the documented open-source "plan usage" flow: first sign-in with `client_id=dynamic_agent_client`, persist the issued client id, a stable `ext_agent_host_id`, and tokens per account; PKCE S256, loopback `http://127.0.0.1:<port>/auth/callback`, scope `openid profile email offline_access resource.invoke chatgpt.tokens.use.direct`, resource `https://api.openai.com/v1`. Do not use the Codex CLI client id.
- Reason: OpenAI documents this flow for open-source and locally hosted apps; borrowing another product's client id is unofficial. The docs mark it as a preview, so behavior may change.
- Confirmed on the live service: dynamic registration, the callback `client_id` parameter, the Responses `instructions` field, model list, and image descriptions. Still unconfirmed: revoke form fields. Access tokens are kept in memory only; the persisted session is client id, host id, e-mail, and refresh token.
- Status: Active.

## 2026-10-06: Mirror Description Into Parameters

- Decision: Saved PNGs carry the description both as iTXt `Description` and as iTXt `Parameters`; the verified save requires the two to match.
- Reason: The Eagle "PNG metadata" plugin only reads `Parameters` (Stable Diffusion style). Tools that parse `Parameters` as an SD prompt will show the description as the prompt text.
- Status: Active.

## 2026-10-06: Support SuperGrok Subscription Sign-In

- Decision: Supersedes the earlier rule to keep SuperGrok disabled. Sign in with the OAuth device code flow (RFC 8628) against auth.x.ai using the shared Grok client id, then call the Responses API on api.x.ai/v1 with the access token. The product treats SuperGrok as a normal supported provider.
- Reason: The user confirmed this flow works for their subscription in other apps. xAI publishes no third-party OAuth documentation, so the client id and endpoints come from the open-source implementations and the existing LingvoLoc module; xAI decides which accounts get tokens and may change this.
- Status: Active.
