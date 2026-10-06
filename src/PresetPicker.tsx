import { useEffect, useId, useMemo, useRef, useState, type KeyboardEvent } from "react";
import type { Preset } from "./lib/contracts";
import { matchesQuery, previewLine } from "./lib/presets";

type Props = {
  presets: readonly Preset[];
  value: string;
  recentIds: readonly string[];
  onChange: (id: string) => void;
  onEdit: () => void;
};

const RECENT_SHOWN = 3;

/** Display order: with no query, up to three recently used presets first, then the rest. */
function arrange(presets: readonly Preset[], query: string, recentIds: readonly string[]): { items: Preset[]; recentCount: number } {
  const matched = presets.filter((item) => matchesQuery(item, query));
  if (query.trim()) return { items: matched, recentCount: 0 };
  const recent = recentIds
    .map((id) => matched.find((item) => item.id === id))
    .filter((item): item is Preset => item !== undefined)
    .slice(0, RECENT_SHOWN);
  const recentSet = new Set(recent.map((item) => item.id));
  return { items: [...recent, ...matched.filter((item) => !recentSet.has(item.id))], recentCount: recent.length };
}

/** Searchable preset chooser; recently used presets come first. */
export function PresetPicker({ presets, value, recentIds, onChange, onEdit }: Props) {
  const [isOpen, setIsOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [activeIndex, setActiveIndex] = useState(0);
  const rootRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const listId = useId();
  const current = presets.find((item) => item.id === value);
  const { items, recentCount } = useMemo(() => arrange(presets, query, recentIds), [presets, query, recentIds]);

  function openList(): void {
    setQuery("");
    setIsOpen(true);
    setActiveIndex(Math.max(0, arrange(presets, "", recentIds).items.findIndex((item) => item.id === value)));
  }

  function close(): void {
    setIsOpen(false);
    triggerRef.current?.focus();
  }

  function choose(preset: Preset): void {
    onChange(preset.id);
    close();
  }

  useEffect(() => {
    if (!isOpen) return;
    inputRef.current?.focus();
    const onPointerDown = (event: PointerEvent) => {
      if (!rootRef.current?.contains(event.target as Node)) setIsOpen(false);
    };
    document.addEventListener("pointerdown", onPointerDown);
    return () => document.removeEventListener("pointerdown", onPointerDown);
  }, [isOpen]);

  useEffect(() => {
    if (!isOpen) return;
    document.getElementById(`${listId}-${activeIndex}`)?.scrollIntoView({ block: "nearest" });
  }, [isOpen, activeIndex, listId]);

  function onKeyDown(event: KeyboardEvent<HTMLInputElement>): void {
    if (event.key === "ArrowDown") {
      event.preventDefault();
      setActiveIndex((index) => Math.min(index + 1, items.length - 1));
    } else if (event.key === "ArrowUp") {
      event.preventDefault();
      setActiveIndex((index) => Math.max(index - 1, 0));
    } else if (event.key === "Enter") {
      event.preventDefault();
      const target = items[activeIndex];
      if (target) choose(target);
    } else if (event.key === "Escape") {
      event.preventDefault();
      close();
    } else if (event.key === "Tab") {
      setIsOpen(false);
    }
  }

  return (
    <div className="field">
      <span className="field-label">Preset</span>
      <div className="select-row picker" ref={rootRef}>
        <button
          ref={triggerRef}
          type="button"
          className="picker-trigger"
          aria-haspopup="listbox"
          aria-expanded={isOpen}
          title={current ? previewLine(current.prompt, 200) : undefined}
          onClick={() => (isOpen ? close() : openList())}
        >
          <span>{current?.name ?? "No preset"}</span>
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true"><path d="m6 9 6 6 6-6" /></svg>
        </button>
        <button type="button" className="icon-button" title="Edit presets" aria-label="Edit presets" onClick={onEdit}>
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true"><path d="M12 20h9" /><path d="M16.5 3.5a2.1 2.1 0 0 1 3 3L7 19l-4 1 1-4Z" /></svg>
        </button>
        {isOpen && (
          <div className="picker-popover">
            <input
              ref={inputRef}
              type="search"
              role="combobox"
              aria-expanded="true"
              aria-controls={listId}
              aria-activedescendant={items[activeIndex] ? `${listId}-${activeIndex}` : undefined}
              aria-label="Search presets"
              placeholder="Search presets"
              autoComplete="off"
              value={query}
              onChange={(event) => { setQuery(event.target.value); setActiveIndex(0); }}
              onKeyDown={onKeyDown}
            />
            <ul id={listId} role="listbox" className="picker-list" aria-label="Presets">
              {items.map((item, index) => (
                <li key={item.id} role="presentation">
                  {recentCount > 0 && index === 0 && <span className="picker-group">Recent</span>}
                  {recentCount > 0 && index === recentCount && <span className="picker-group">All presets</span>}
                  <div
                    id={`${listId}-${index}`}
                    role="option"
                    aria-selected={item.id === value}
                    data-active={index === activeIndex}
                    className={index === activeIndex ? "picker-option active" : "picker-option"}
                    onPointerEnter={() => setActiveIndex(index)}
                    onClick={() => choose(item)}
                  >
                    <span className="picker-option-name">{item.name}{item.id === value && <em className="preset-badge">Selected</em>}</span>
                    <span className="picker-option-preview">{previewLine(item.prompt)}</span>
                  </div>
                </li>
              ))}
              {items.length === 0 && <li className="muted picker-empty" role="presentation">{presets.length === 0 ? "No presets. Open the editor to add one." : "No presets match."}</li>}
            </ul>
          </div>
        )}
      </div>
    </div>
  );
}
