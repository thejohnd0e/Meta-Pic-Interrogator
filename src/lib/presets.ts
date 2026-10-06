import type { Preset } from "./contracts";

const RECENT_KEY = "metapic-recent-presets";
const RECENT_LIMIT = 5;

/** Same order the backend uses: case-insensitive name, then id. */
export function sortPresets(items: readonly Preset[]): Preset[] {
  return [...items].sort((a, b) => {
    const left = a.name.toLowerCase();
    const right = b.name.toLowerCase();
    if (left !== right) return left < right ? -1 : 1;
    return a.id < b.id ? -1 : a.id > b.id ? 1 : 0;
  });
}

export function matchesQuery(preset: Preset, query: string): boolean {
  const needle = query.trim().toLowerCase();
  if (!needle) return true;
  return preset.name.toLowerCase().includes(needle) || preset.prompt.toLowerCase().includes(needle);
}

/** First non-empty line of a prompt, without Markdown heading marks, for list previews. */
export function previewLine(prompt: string, max = 120): string {
  const line = prompt.split(/\r?\n/).map((item) => item.replace(/^\s*#+\s*/, "").trim()).find(Boolean) ?? "";
  return line.length > max ? `${line.slice(0, max).trimEnd()}...` : line;
}

/** Rough token estimate (about four characters per token); only a size hint. */
export function estimateTokens(text: string): number {
  return Math.ceil(text.length / 4);
}

export function safeFileName(name: string): string {
  const cleaned = name.replace(/[<>:"/\\|?*\u0000-\u001f]/g, "").trim().replace(/[. ]+$/, "");
  return cleaned.slice(0, 80) || "preset";
}

export function readRecentPresets(): string[] {
  try {
    const parsed: unknown = JSON.parse(localStorage.getItem(RECENT_KEY) ?? "[]");
    return Array.isArray(parsed) ? parsed.filter((item): item is string => typeof item === "string").slice(0, RECENT_LIMIT) : [];
  } catch {
    return [];
  }
}

/** Moves `id` to the front of the recently used list and remembers it. */
export function rememberPreset(current: readonly string[], id: string): string[] {
  const next = [id, ...current.filter((item) => item !== id)].slice(0, RECENT_LIMIT);
  try {
    localStorage.setItem(RECENT_KEY, JSON.stringify(next));
  } catch {
    // Storage can be unavailable; the list then lasts for this session only.
  }
  return next;
}
