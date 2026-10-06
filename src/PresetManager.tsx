import { useEffect, useMemo, useRef, useState, type Dispatch, type SetStateAction } from "react";
import { open, save } from "@tauri-apps/plugin-dialog";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { commands } from "./lib/commands";
import { decodeAppError, type Preset } from "./lib/contracts";
import { estimateTokens, matchesQuery, previewLine, safeFileName, sortPresets } from "./lib/presets";

type SaveState = "idle" | "editing" | "saving" | "saved" | "invalid" | "error";
type Draft = { id: string; name: string; prompt: string };

type Props = {
  presets: readonly Preset[];
  setPresets: Dispatch<SetStateAction<Preset[]>>;
  activeId: string;
  onUse: (id: string) => void;
};

const AUTOSAVE_MS = 700;
const isDesktop = () => "__TAURI_INTERNALS__" in window;
const errorText = (error: unknown, fallback: string) => decodeAppError(error).message ?? fallback;

const saveLabels: Record<SaveState, string> = {
  idle: "",
  editing: "Unsaved changes...",
  saving: "Saving...",
  saved: "Saved to file",
  invalid: "Not saved: name and prompt cannot be empty",
  error: "Could not save",
};

export function PresetManager({ presets, setPresets, activeId, onUse }: Props) {
  const [selectedId, setSelectedId] = useState(() => (presets.some((item) => item.id === activeId) ? activeId : (presets[0]?.id ?? "")));
  const [draft, setDraft] = useState<Draft | null>(() => {
    const first = presets.find((item) => item.id === activeId) ?? presets[0];
    return first ? { ...first } : null;
  });
  const [query, setQuery] = useState("");
  const [saveState, setSaveState] = useState<SaveState>("idle");
  const [message, setMessage] = useState("");
  const [busy, setBusy] = useState(false);
  const [dragging, setDragging] = useState(false);
  const nameRef = useRef<HTMLInputElement>(null);
  const focusNameRef = useRef(false);
  const pendingRef = useRef<Draft | null>(null);
  const timerRef = useRef<number | undefined>(undefined);
  const flushRef = useRef<() => Promise<void>>(async () => undefined);
  const importRef = useRef<(paths: string[]) => Promise<void>>(async () => undefined);

  const visible = useMemo(() => presets.filter((item) => matchesQuery(item, query)), [presets, query]);
  const selected = presets.find((item) => item.id === selectedId) ?? null;

  async function flush(): Promise<void> {
    window.clearTimeout(timerRef.current);
    const pending = pendingRef.current;
    if (!pending) return;
    pendingRef.current = null;
    if (!pending.name.trim() || !pending.prompt.trim()) {
      setSaveState("invalid");
      return;
    }
    if (!isDesktop()) {
      setPresets((current) => sortPresets(current.map((item) => (item.id === pending.id ? { ...pending } : item))));
      setSaveState("saved");
      return;
    }
    setSaveState("saving");
    try {
      const saved = await commands.updatePreset(pending);
      setPresets((current) => sortPresets(current.map((item) => (item.id === saved.id ? saved : item))));
      setSaveState("saved");
    } catch (error) {
      setSaveState("error");
      setMessage(errorText(error, "Could not save preset."));
    }
  }
  flushRef.current = flush;

  // Leaving the editor (or the whole view) must not lose the last edit.
  useEffect(() => () => void flushRef.current(), []);

  useEffect(() => {
    if (!focusNameRef.current) return;
    focusNameRef.current = false;
    nameRef.current?.focus();
    nameRef.current?.select();
  }, [selectedId]);

  function edit(change: Partial<Draft>): void {
    if (!draft) return;
    const next = { ...draft, ...change };
    setDraft(next);
    pendingRef.current = next;
    setSaveState("editing");
    window.clearTimeout(timerRef.current);
    timerRef.current = window.setTimeout(() => void flushRef.current(), AUTOSAVE_MS);
  }

  function show(preset: Preset | null, focusName = false): void {
    pendingRef.current = null;
    window.clearTimeout(timerRef.current);
    focusNameRef.current = focusName;
    setSelectedId(preset?.id ?? "");
    setDraft(preset ? { ...preset } : null);
    setSaveState("idle");
  }

  function select(preset: Preset): void {
    if (preset.id === selectedId) return;
    void flush();
    show(preset);
  }

  async function addPreset(name: string, prompt: string): Promise<void> {
    await flush();
    setBusy(true);
    try {
      const created = isDesktop()
        ? await commands.createPreset({ id: "", name, prompt })
        : { id: `local-${Date.now()}`, name, prompt };
      setPresets((current) => sortPresets([...current, created]));
      setQuery("");
      show(created, true);
      setMessage("");
    } catch (error) {
      setMessage(errorText(error, "Could not create preset."));
    } finally {
      setBusy(false);
    }
  }

  async function removePreset(): Promise<void> {
    if (!selected) return;
    if (!window.confirm(`Delete "${selected.name}"? Its .md file is removed from the presets folder.`)) return;
    pendingRef.current = null;
    window.clearTimeout(timerRef.current);
    setBusy(true);
    try {
      if (isDesktop()) await commands.deletePreset(selected.id);
      const index = presets.findIndex((item) => item.id === selected.id);
      const remaining = presets.filter((item) => item.id !== selected.id);
      setPresets(remaining);
      show(remaining[Math.min(index, remaining.length - 1)] ?? null);
      setMessage(`Deleted "${selected.name}".`);
    } catch (error) {
      setMessage(errorText(error, "Could not delete preset."));
    } finally {
      setBusy(false);
    }
  }

  async function importPaths(paths: string[]): Promise<void> {
    if (paths.length === 0) return;
    await flush();
    setBusy(true);
    try {
      const created = await commands.importPresets(paths);
      setPresets((current) => sortPresets([...current, ...created]));
      setQuery("");
      const first = created[0];
      if (first) show(first);
      setMessage(`Imported ${created.length} of ${paths.length} file${paths.length === 1 ? "" : "s"}.`);
    } catch (error) {
      setMessage(errorText(error, "Could not import files."));
    } finally {
      setBusy(false);
    }
  }
  importRef.current = importPaths;

  async function chooseImport(): Promise<void> {
    if (!isDesktop()) {
      setMessage("Open the desktop app to import files.");
      return;
    }
    const picked = await open({ multiple: true, directory: false, filters: [{ name: "Markdown or text", extensions: ["md", "markdown", "txt"] }] });
    if (Array.isArray(picked)) await importPaths(picked);
  }

  async function exportPreset(): Promise<void> {
    if (!selected || !isDesktop()) return;
    await flush();
    const destination = await save({ defaultPath: `${safeFileName(draft?.name ?? selected.name)}.md`, filters: [{ name: "Markdown", extensions: ["md"] }] });
    if (!destination) return;
    try {
      await commands.exportPreset(selected.id, destination);
      setMessage("Exported.");
    } catch (error) {
      setMessage(errorText(error, "Could not export preset."));
    }
  }

  async function reload(): Promise<void> {
    if (!isDesktop()) return;
    await flush();
    try {
      const fresh = sortPresets(await commands.listPresets());
      setPresets(fresh);
      const current = fresh.find((item) => item.id === selectedId);
      if (current) {
        setDraft({ ...current });
        setSaveState("idle");
      } else {
        show(fresh[0] ?? null);
      }
      setMessage(`Reloaded ${fresh.length} preset${fresh.length === 1 ? "" : "s"} from the folder.`);
    } catch (error) {
      setMessage(errorText(error, "Could not reload presets."));
    }
  }

  async function openFolder(): Promise<void> {
    if (!isDesktop()) return;
    try {
      await commands.openPresetsFolder();
    } catch (error) {
      setMessage(errorText(error, "Could not open the folder."));
    }
  }

  // Dropping .md/.txt files anywhere on the window imports them.
  useEffect(() => {
    if (!isDesktop()) return;
    let mounted = true;
    let unlisten: (() => void) | undefined;
    void getCurrentWebviewWindow().onDragDropEvent((event) => {
      if (!mounted) return;
      if (event.payload.type === "enter" || event.payload.type === "over") setDragging(true);
      else if (event.payload.type === "leave") setDragging(false);
      else if (event.payload.type === "drop") {
        setDragging(false);
        void importRef.current(event.payload.paths);
      }
    }).then((dispose) => {
      if (mounted) unlisten = dispose;
      else dispose();
    });
    return () => {
      mounted = false;
      unlisten?.();
    };
  }, []);

  const active = selected?.id === activeId;
  const promptText = draft?.prompt ?? "";

  return (
    <section className={dragging ? "preset-manager dragging" : "preset-manager"}>
      <aside className="preset-sidebar">
        <input type="search" value={query} onChange={(event) => setQuery(event.target.value)} placeholder="Search names and prompts" aria-label="Search presets" />
        <div className="preset-toolbar">
          <button className="primary-button" disabled={busy} onClick={() => void addPreset("New preset", "Describe the image.")}>New</button>
          <button className="secondary-button" disabled={busy} onClick={() => void chooseImport()}>Import .md</button>
        </div>
        <span className="preset-count" aria-live="polite">{visible.length === presets.length ? `${presets.length} presets` : `${visible.length} of ${presets.length} presets`}</span>
        <ul className="preset-list">
          {visible.map((item) => (
            <li key={item.id}>
              <button className={item.id === selectedId ? "preset-item selected" : "preset-item"} aria-current={item.id === selectedId} onClick={() => select(item)}>
                <span className="preset-item-name">{item.name}{item.id === activeId && <em className="preset-badge">In use</em>}</span>
                <span className="preset-item-preview">{previewLine(item.id === selectedId && draft ? draft.prompt : item.prompt)}</span>
              </button>
            </li>
          ))}
          {visible.length === 0 && <li className="muted preset-empty">{presets.length === 0 ? "No presets yet. Create one or import .md files." : "No presets match your search."}</li>}
        </ul>
        <div className="preset-sidebar-foot">
          <button className="text-button" onClick={() => void openFolder()}>Open folder</button>
          <button className="text-button" onClick={() => void reload()}>Reload</button>
        </div>
      </aside>

      <div className="preset-editor">
        {draft && selected ? (
          <>
            <div className="preset-editor-head">
              <label className="grow">Name<input ref={nameRef} value={draft.name} onChange={(event) => edit({ name: event.target.value })} maxLength={120} autoComplete="off" /></label>
              <div className="preset-toolbar">
                <button className="primary-button" disabled={active} onClick={() => onUse(selected.id)}>{active ? "In use" : "Use this preset"}</button>
                <button className="secondary-button" disabled={busy} onClick={() => void addPreset(`${draft.name.trim() || selected.name} copy`, draft.prompt)}>Duplicate</button>
                <button className="secondary-button" disabled={busy} onClick={() => void exportPreset()}>Export .md</button>
                <button className="text-button" disabled={busy} onClick={() => void removePreset()}>Delete</button>
              </div>
            </div>
            <label className="preset-prompt">Prompt (plain text or Markdown, sent exactly as written)
              <textarea value={draft.prompt} onChange={(event) => edit({ prompt: event.target.value })} spellCheck={false} />
            </label>
            <div className="preset-foot">
              <span className={saveState === "invalid" || saveState === "error" ? "warning" : "muted"} role="status" aria-live="polite">{saveLabels[saveState]}</span>
              <span className="muted">{promptText.length.toLocaleString()} characters, about {estimateTokens(promptText).toLocaleString()} tokens</span>
            </div>
          </>
        ) : (
          <p className="muted preset-empty">Select a preset, or create one. You can also drop .md files here to import them.</p>
        )}
        {message && <p className="form-notice" role="status" aria-live="polite">{message}</p>}
        {dragging && <div className="preset-drop" aria-hidden="true">Drop .md or .txt files to import</div>}
      </div>
    </section>
  );
}
