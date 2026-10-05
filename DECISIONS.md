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
