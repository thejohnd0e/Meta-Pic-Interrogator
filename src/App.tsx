import { useRef, useState } from "react";
import { commands } from "./lib/commands";
import "./App.css";

type Preset = { id: string; name: string; prompt: string };

const initialPresets: Preset[] = [
  { id: "concise", name: "Concise", prompt: "Describe the image clearly and briefly." },
  { id: "detailed", name: "Detailed", prompt: "Describe the image with useful visual detail." },
];

function App() {
  const inputRef = useRef<HTMLInputElement>(null);
  const [file, setFile] = useState<File | null>(null);
  const [preview, setPreview] = useState("");
  const [description, setDescription] = useState("");
  const [provider, setProvider] = useState("openai");
  const [model, setModel] = useState("vision");
  const [presetId, setPresetId] = useState("concise");
  const [presets, setPresets] = useState(initialPresets);
  const [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState("Ready for an image");
  const [settings, setSettings] = useState(false);

  function selectFile(next: File | undefined): void {
    if (!next) return;
    const url = URL.createObjectURL(next);
    setFile(next);
    setPreview(url);
    setDescription("");
    setNotice(`${next.name} loaded. Ready to describe.`);
  }

  async function describe(): Promise<void> {
    if (!file) return;
    const preset = presets.find((item) => item.id === presetId) ?? presets[0];
    if (!preset) return;
    setBusy(true);
    setNotice("Sending image to the selected vision provider...");
    try {
      const result = await commands.describeImage({ path: file.name }, { providerId: provider, modelId: model, endpoint: null }, preset);
      setDescription(result.text);
      setNotice("Description ready to review.");
    } catch (error) {
      setNotice(error instanceof Error ? error.message : "Description failed; your draft was preserved.");
    } finally {
      setBusy(false);
    }
  }

  async function save(): Promise<void> {
    if (!description.trim() || !file) return;
    setBusy(true);
    setNotice("Saving a verified PNG copy...");
    try {
      await commands.savePngCopy({ sourcePath: file.name, destinationPath: `${file.name}-described.png`, description, provenance: { schemaVersion: 1, provider, model, presetId, presetName: presets.find((item) => item.id === presetId)?.name ?? "Preset", presetPrompt: presets.find((item) => item.id === presetId)?.prompt ?? "", createdAtUtc: new Date().toISOString(), appVersion: "0.1.0" } });
      setNotice("PNG copy saved.");
    } catch (error) {
      setNotice(error instanceof Error ? error.message : "Save failed; your draft was preserved.");
    } finally {
      setBusy(false);
    }
  }

  return (
    <main className="app-shell">
      <header className="topbar"><div><span className="overline">MetaPic / Interrogator</span><h1>Image, then evidence.</h1></div><button className="text-button" onClick={() => setSettings(!settings)}>{settings ? "Back to workspace" : "Settings"}</button></header>
      {settings ? <section className="settings-panel"><span className="overline">Provider settings</span><h2>Connection surface</h2><p className="muted">API credentials remain in Windows Credential Manager. Subscription providers stay disabled until a vision probe succeeds.</p><div className="provider-grid">{["OpenAI", "Anthropic", "Gemini", "OpenRouter", "xAI", "DeepSeek"].map((name) => <article className="provider-card" key={name}><strong>{name}</strong><span className={name === "xAI" || name === "DeepSeek" ? "warning" : "ready"}>{name === "xAI" || name === "DeepSeek" ? "Vision probe required" : "Ready to configure"}</span></article>)}</div><h2>Presets</h2>{presets.map((item) => <label className="preset-row" key={item.id}><span>{item.name}</span><input value={item.prompt} onChange={(event) => setPresets(presets.map((preset) => preset.id === item.id ? { ...preset, prompt: event.target.value } : preset))} /></label>)}</section> : <section className="workspace"><div className="import-panel"><div className="drop-zone" onDragOver={(event) => event.preventDefault()} onDrop={(event) => { event.preventDefault(); selectFile(event.dataTransfer.files[0]); }}><input ref={inputRef} type="file" accept="image/png,image/jpeg,image/webp,image/bmp,image/tiff" hidden onChange={(event) => selectFile(event.target.files?.[0])} />{preview ? <img src={preview} alt="Selected image preview" /> : <><span className="drop-mark">01</span><h2>Bring one image into focus.</h2><p>PNG, JPEG, WebP, BMP, or TIFF. The source is never modified.</p></>}<button className="primary-button" onClick={() => inputRef.current?.click()}>{file ? "Choose another image" : "Choose image"}</button></div>{file && <div className="file-meta"><span>{file.name}</span><span>{Math.round(file.size / 1024)} KB</span></div>}</div><div className="detail-panel"><div className="panel-heading"><span className="overline">02 / Describe</span><span className={busy ? "status busy" : "status"}>{notice}</span></div><div className="controls"><label>Provider<select value={provider} onChange={(event) => setProvider(event.target.value)}><option value="openai">OpenAI</option><option value="anthropic">Anthropic</option><option value="gemini">Gemini</option><option value="openrouter">OpenRouter</option></select></label><label>Model<select value={model} onChange={(event) => setModel(event.target.value)}><option value="vision">Vision model</option><option value="custom">Custom vision model</option></select></label><label>Preset<select value={presetId} onChange={(event) => setPresetId(event.target.value)}>{presets.map((item) => <option value={item.id} key={item.id}>{item.name}</option>)}</select></label></div><label className="editor-label">Description<textarea value={description} onChange={(event) => setDescription(event.target.value)} placeholder="Your generated description will appear here. You can edit it before saving." /></label><div className="actions"><button className="primary-button" disabled={!file || busy} onClick={() => void describe()}>{busy ? "Working..." : "Describe image"}</button><button className="secondary-button" disabled={!description.trim() || busy} onClick={() => void save()}>Save PNG copy</button></div></div></section>}
    </main>
  );
}

export default App;
