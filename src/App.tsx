import { useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { open, save as saveDialog } from "@tauri-apps/plugin-dialog";
import { readFile } from "@tauri-apps/plugin-fs";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { commands } from "./lib/commands";
import { decodeAppError, type ChatGptStatus, type VisionModel } from "./lib/contracts";
import "./App.css";

type Preset = { id: string; name: string; prompt: string };
type DescriptionStarted = { requestId: number };
type DescriptionDelta = { requestId: number; text: string };
type ProviderOption = { id: string; name: string; mode: "api" | "subscription"; vision: boolean; endpoint: string };

const initialPresets: Preset[] = [
  { id: "concise", name: "Concise", prompt: "Describe the image clearly and briefly." },
  { id: "detailed", name: "Detailed", prompt: "Describe the image with useful visual detail." },
];

const providerOptions: ProviderOption[] = [
  { id: "openai", name: "OpenAI API", mode: "api", vision: true, endpoint: "https://api.openai.com" },
  { id: "openai-compatible", name: "OpenAI-compatible", mode: "api", vision: true, endpoint: "https://" },
  { id: "anthropic", name: "Anthropic API", mode: "api", vision: false, endpoint: "https://api.anthropic.com" },
  { id: "gemini", name: "Google Gemini API", mode: "api", vision: false, endpoint: "https://generativelanguage.googleapis.com" },
  { id: "openrouter", name: "OpenRouter API", mode: "api", vision: false, endpoint: "https://openrouter.ai/api" },
  { id: "chatgpt", name: "ChatGPT Plus", mode: "subscription", vision: true, endpoint: "" },
  { id: "xai", name: "SuperGrok", mode: "subscription", vision: false, endpoint: "" },
];

const supportedExtensions = ["png", "jpg", "jpeg", "webp", "bmp", "tif", "tiff"];
const supportedMimeTypes = ["image/png", "image/jpeg", "image/webp", "image/bmp", "image/tiff"];

function hasSupportedImageType(name: string, type = ""): boolean {
  const extension = name.split(".").pop()?.toLowerCase() ?? "";
  return supportedExtensions.includes(extension) && (!type || supportedMimeTypes.includes(type));
}

function mimeForPath(path: string): string {
  const extension = path.split(".").pop()?.toLowerCase();
  return extension === "jpg" || extension === "jpeg" ? "image/jpeg" : extension === "webp" ? "image/webp" : "image/png";
}

function App() {
  const inputRef = useRef<HTMLInputElement>(null);
  const activeRequestIdRef = useRef<number | null>(null);
  const [file, setFile] = useState<File | null>(null);
  const [nativePath, setNativePath] = useState("");
  const [preview, setPreview] = useState("");
  const [description, setDescription] = useState("");
  const [provider, setProvider] = useState("openai");
  const [model, setModel] = useState("vision");
  const [endpoint, setEndpoint] = useState("https://api.openai.com");
  const [credential, setCredential] = useState("");
  const [credentialConfigured, setCredentialConfigured] = useState(false);
  const [settingsNotice, setSettingsNotice] = useState("");
  const [settingsBusy, setSettingsBusy] = useState(false);
  const [presetId, setPresetId] = useState("concise");
  const [presets, setPresets] = useState(initialPresets);
  const [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState("Ready for an image");
  const [settings, setSettings] = useState(false);
  const [chatgpt, setChatgpt] = useState<ChatGptStatus>({ configured: false, email: null });
  const [chatgptBusy, setChatgptBusy] = useState(false);
  const [chatgptModels, setChatgptModels] = useState<readonly VisionModel[]>([]);

  useEffect(() => {
    if (!("__TAURI_INTERNALS__" in window)) return;
    void Promise.all([commands.loadSettings(), commands.listPresets()]).then(([saved, storedPresets]) => {
      if (saved.providerId) setProvider(saved.providerId);
      if (saved.modelId) setModel(saved.modelId);
      if (saved.endpoint) setEndpoint(saved.endpoint);
      if (saved.presetId) setPresetId(saved.presetId);
      if (storedPresets.length > 0) setPresets(storedPresets as Preset[]);
    }).catch(() => setSettingsNotice("Could not load saved settings."));
  }, []);

  useEffect(() => {
    if (!("__TAURI_INTERNALS__" in window)) return;
    void commands.chatgptStatus().then(setChatgpt).catch(() => setChatgpt({ configured: false, email: null }));
  }, []);

  useEffect(() => {
    if (provider !== "chatgpt" || !chatgpt.configured || chatgptModels.length > 0) return;
    void loadChatgptModels();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [provider, chatgpt.configured]);

  useEffect(() => {
    if (!("__TAURI_INTERNALS__" in window)) return;
    void commands.credentialStatus(provider).then(setCredentialConfigured).catch(() => setCredentialConfigured(false));
  }, [provider]);

  useEffect(() => {
    if (!("__TAURI_INTERNALS__" in window)) return;
    let mounted = true;
    const cleanups: Array<() => void> = [];
    void listen<DescriptionStarted>("description-started", (event) => {
      activeRequestIdRef.current = event.payload.requestId;
      setDescription("");
      setNotice("Receiving description...");
    }).then((dispose) => {
      if (mounted) cleanups.push(dispose);
      else dispose();
    });
    void listen<DescriptionDelta>("description-delta", (event) => {
      if (event.payload.requestId !== activeRequestIdRef.current) return;
      setDescription((current) => current + event.payload.text);
    }).then((dispose) => {
      if (mounted) cleanups.push(dispose);
      else dispose();
    });
    return () => {
      mounted = false;
      cleanups.forEach((dispose) => dispose());
    };
  }, []);

  useEffect(() => {
    if (!("__TAURI_INTERNALS__" in window)) return;
    let mounted = true;
    let unlisten: (() => void) | undefined;
    void getCurrentWebviewWindow().onDragDropEvent((event) => {
      if (!mounted) return;
      if (event.payload.type === "enter" || event.payload.type === "over") {
        setNotice("Drop one supported image to load it.");
      } else if (event.payload.type === "drop" && event.payload.paths[0]) {
        void selectNativePath(event.payload.paths[0]);
      } else if (event.payload.type === "leave") {
        setNotice((current) => current === "Drop one supported image to load it." ? "Ready for an image" : current);
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

  function selectFile(next: File | undefined): void {
    if (!next) return;
    if (!hasSupportedImageType(next.name, next.type)) {
      setNotice("Unsupported image type.");
      return;
    }
    if (description.trim() && !window.confirm("Replace the current image and discard this description?")) return;
    setFile(next);
    setNativePath("");
    const reader = new FileReader();
    reader.onload = () => setPreview(typeof reader.result === "string" ? reader.result : "");
    reader.readAsDataURL(next);
    setDescription("");
    setNotice("__TAURI_INTERNALS__" in window ? "Use Choose image for a native path." : "Preview loaded. Open the desktop app to describe or save.");
  }

  async function selectNativePath(path: string): Promise<void> {
    if (!hasSupportedImageType(path)) {
      setNotice("Unsupported image type.");
      return;
    }
    if (description.trim() && !window.confirm("Replace the current image and discard this description?")) return;
    setFile(null);
    setNativePath(path);
    setDescription("");
    const name = path.split(/[\\/]/).pop() ?? path;
    try {
      const bytes = await readFile(path);
      setPreview(URL.createObjectURL(new Blob([bytes], { type: mimeForPath(path) })));
      setNotice(`${name} loaded. Ready to describe.`);
    } catch {
      setPreview("");
      setNotice(`${name} loaded. Preview unavailable; ready to describe.`);
    }
  }

  async function chooseImage(): Promise<void> {
    if (!("__TAURI_INTERNALS__" in window)) {
      inputRef.current?.click();
      return;
    }
    const selected = await open({
      multiple: false,
      directory: false,
      filters: [{ name: "Images", extensions: ["png", "jpg", "jpeg", "webp", "bmp", "tif", "tiff"] }],
    });
    if (typeof selected === "string") void selectNativePath(selected);
  }

  function selectProvider(providerId: string): void {
    const next = providerOptions.find((item) => item.id === providerId);
    setProvider(providerId);
    if (next?.endpoint) setEndpoint(next.endpoint);
    if (providerId === "chatgpt") {
      setSettingsNotice(chatgpt.configured ? "" : "Sign in with ChatGPT below to use your plan.");
    } else if (next?.mode === "subscription") {
      setSettingsNotice("Subscription sign-in is not available for this provider.");
    } else {
      setSettingsNotice("");
    }
  }

  async function saveProviderSettings(): Promise<void> {
    if (!("__TAURI_INTERNALS__" in window)) {
      setSettingsNotice("Open the desktop app to persist provider settings.");
      return;
    }
    setSettingsBusy(true);
    try {
      await commands.saveSettings({ schemaVersion: 1, providerId: provider, modelId: model, endpoint: endpoint || null, presetId });
      if (credential.trim()) {
        await commands.setCredential(provider, credential.trim());
        setCredential("");
        setCredentialConfigured(true);
      }
      setSettingsNotice("Provider settings saved securely.");
    } catch (error) {
      setSettingsNotice(error instanceof Error ? error.message : "Could not save provider settings.");
    } finally {
      setSettingsBusy(false);
    }
  }

  async function removeProviderCredential(): Promise<void> {
    if (!("__TAURI_INTERNALS__" in window)) return;
    setSettingsBusy(true);
    try {
      await commands.deleteCredential(provider);
      setCredentialConfigured(false);
      setSettingsNotice("Stored credential removed.");
    } catch (error) {
      setSettingsNotice(error instanceof Error ? error.message : "Could not remove credential.");
    } finally {
      setSettingsBusy(false);
    }
  }

  async function chatgptSignIn(): Promise<void> {
    if (!("__TAURI_INTERNALS__" in window)) {
      setSettingsNotice("Open the desktop app to sign in.");
      return;
    }
    setChatgptBusy(true);
    setSettingsNotice("Finish signing in in your browser...");
    try {
      const status = await commands.chatgptSignIn();
      setChatgpt(status);
      setSettingsNotice("Signed in with ChatGPT.");
      await loadChatgptModels();
    } catch (error) {
      setSettingsNotice(decodeAppError(error).message ?? "ChatGPT sign-in failed.");
    } finally {
      setChatgptBusy(false);
    }
  }

  async function chatgptSignOut(): Promise<void> {
    setChatgptBusy(true);
    try {
      await commands.chatgptSignOut();
      setChatgpt({ configured: false, email: null });
      setChatgptModels([]);
      setSettingsNotice("Signed out of ChatGPT.");
    } catch (error) {
      setSettingsNotice(decodeAppError(error).message ?? "Could not sign out.");
    } finally {
      setChatgptBusy(false);
    }
  }

  async function loadChatgptModels(): Promise<void> {
    try {
      const models = await commands.chatgptModels();
      setChatgptModels(models);
      const first = models[0];
      if (first && !models.some((item) => item.id === model)) setModel(first.id);
    } catch (error) {
      const message = decodeAppError(error).message ?? "Could not load ChatGPT models.";
      setSettingsNotice(message);
      setNotice(message);
    }
  }

  async function persistPreset(preset: Preset): Promise<void> {
    if (!("__TAURI_INTERNALS__" in window)) return;
    try {
      await commands.updatePreset(preset);
      setSettingsNotice("Preset saved.");
    } catch (error) {
      setSettingsNotice(error instanceof Error ? error.message : "Could not save preset.");
    }
  }

  async function describe(): Promise<void> {
    const sourcePath = nativePath;
    if (!sourcePath) {
      setNotice("Open the desktop app to describe a selected image.");
      return;
    }
    const selectedProvider = providerOptions.find((item) => item.id === provider);
    if (!selectedProvider?.vision) {
      setNotice(`${selectedProvider?.name ?? "This provider"} is not available for image descriptions yet.`);
      return;
    }
    const preset = presets.find((item) => item.id === presetId) ?? presets[0];
    if (!preset) return;
    setBusy(true);
    setDescription("");
    setNotice("Sending image to the selected vision provider...");
    try {
      const result = await commands.describeImage(
        { path: sourcePath },
        { providerId: provider, modelId: model, endpoint: endpoint || null },
        preset,
      );
      setDescription(result.text);
      setNotice("Description ready to review.");
    } catch (error) {
      setNotice(error instanceof Error ? error.message : "Description failed; your draft was preserved.");
    } finally {
      setBusy(false);
    }
  }

  async function save(): Promise<void> {
    const sourcePath = nativePath;
    if (!description.trim() || !sourcePath) return;
    const defaultDestination = `${sourcePath.replace(/\.[^/.]+$/, "")}-described.png`;
    setBusy(true);
    setNotice("Saving a verified PNG copy...");
    try {
      const destinationPath = nativePath
        ? await saveDialog({ defaultPath: defaultDestination, filters: [{ name: "PNG image", extensions: ["png"] }] })
        : defaultDestination;
      if (!destinationPath) {
        setNotice("Save cancelled.");
        return;
      }
      await commands.savePngCopy({
        sourcePath,
        destinationPath,
        description,
        provenance: {
          schemaVersion: 1,
          provider,
          model,
          presetId,
          presetName: presets.find((item) => item.id === presetId)?.name ?? "Preset",
          presetPrompt: presets.find((item) => item.id === presetId)?.prompt ?? "",
          createdAtUtc: new Date().toISOString(),
          appVersion: "0.1.0",
        },
      });
      setNotice("PNG copy saved.");
    } catch (error) {
      setNotice(error instanceof Error ? error.message : "Save failed; your draft was preserved.");
    } finally {
      setBusy(false);
    }
  }

  return (
    <main className="app-shell">
      <header className="topbar">
        <div><span className="overline">MetaPic / Interrogator</span><h1>Image, then evidence.</h1></div>
        <div className="topbar-actions">
          {busy && <button className="text-button" onClick={() => void commands.cancelDescription()}>Cancel</button>}
          <button className="text-button" onClick={() => setSettings(!settings)}>{settings ? "Back to workspace" : "Settings"}</button>
        </div>
      </header>
      {settings ? (
        <section className="settings-panel">
          <span className="overline">Access settings</span>
          <h2>Connect your account</h2>
          <p className="muted">Subscription access is the primary path. API credentials are stored in Windows Credential Manager and are only used for providers that expose an official API.</p>

          <div className="settings-section subscription-section">
            <div className="section-heading"><div><span className="overline">Subscription access</span><h3>Use an existing plan</h3></div><span className="status busy">OAuth setup required</span></div>
            <div className="subscription-grid">
              {providerOptions.filter((item) => item.mode === "subscription").map((item) => <article className="provider-card" key={item.id}>
                <strong>{item.name}</strong>
                {item.id === "chatgpt"
                  ? <>
                    <span className={chatgpt.configured ? "status" : "warning"}>{chatgpt.configured ? `Signed in${chatgpt.email ? ` as ${chatgpt.email}` : ""}` : "Not signed in"}</span>
                    <p>Uses your ChatGPT plan through the official Sign in with ChatGPT flow. Sign-in opens in your browser.</p>
                    {chatgptBusy
                      ? <button className="text-button" onClick={() => void commands.chatgptCancelSignIn()}>Cancel sign-in</button>
                      : chatgpt.configured
                        ? <div className="action-row"><button className="text-button" onClick={() => void loadChatgptModels()}>Refresh models</button><button className="text-button" onClick={() => void chatgptSignOut()}>Sign out</button></div>
                        : <button className="primary-button" onClick={() => void chatgptSignIn()}>Sign in with ChatGPT</button>}
                  </>
                  : <>
                    <span className="warning">Official sign-in not available</span>
                    <p>SuperGrok subscription OAuth is not exposed as a public inference API by xAI.</p>
                    <button className="text-button" disabled onClick={() => undefined}>Sign in unavailable</button>
                  </>}
              </article>)}
            </div>
          </div>

          <div className="settings-section">
            <div className="section-heading"><div><span className="overline">API fallback</span><h3>Provider connection</h3></div><span className={credentialConfigured ? "status" : "status busy"}>{credentialConfigured ? "Credential stored" : "Credential not set"}</span></div>
            <div className="settings-form">
              <label>Provider<select value={provider} onChange={(event) => selectProvider(event.target.value)}>{providerOptions.map((item) => <option value={item.id} key={item.id}>{item.name}</option>)}</select></label>
              {provider === "chatgpt" && chatgptModels.length > 0
                ? <label>Model<select value={model} onChange={(event) => setModel(event.target.value)}>{chatgptModels.map((item) => <option value={item.id} key={item.id}>{item.displayName}</option>)}</select></label>
                : <label>Model<input value={model} onChange={(event) => setModel(event.target.value)} placeholder="gpt-4.1-mini" /></label>}
              <label>Endpoint<input value={endpoint} onChange={(event) => setEndpoint(event.target.value)} placeholder="https://api.openai.com" disabled={providerOptions.find((item) => item.id === provider)?.mode === "subscription"} /></label>
              <label>API key<input type="password" value={credential} onChange={(event) => setCredential(event.target.value)} placeholder={credentialConfigured ? "Stored securely; enter to replace" : "Enter provider API key"} autoComplete="off" disabled={providerOptions.find((item) => item.id === provider)?.mode === "subscription"} /></label>
            </div>
            <div className="action-row"><button className="primary-button" disabled={settingsBusy} onClick={() => void saveProviderSettings()}>{settingsBusy ? "Saving..." : "Save provider settings"}</button>{credentialConfigured && <button className="text-button" disabled={settingsBusy} onClick={() => void removeProviderCredential()}>Remove stored key</button>}</div>
            {settingsNotice && <p className="form-notice" role="status" aria-live="polite">{settingsNotice}</p>}
          </div>

          <div className="settings-section">
            <div className="section-heading"><div><span className="overline">Prompt library</span><h3>Presets</h3></div></div>
            {presets.map((item) => <label className="preset-row" key={item.id}><span>{item.name}</span><input value={item.prompt} onChange={(event) => setPresets(presets.map((preset) => preset.id === item.id ? { ...preset, prompt: event.target.value } : preset))} onBlur={() => void persistPreset(presets.find((preset) => preset.id === item.id) ?? item)} /></label>)}
          </div>
        </section>
      ) : (
        <section className="workspace">
          <div className="import-panel">
            <div className="drop-zone" tabIndex={0} onKeyDown={(event) => { if (event.key === "Enter" || event.key === " ") { event.preventDefault(); void chooseImage(); } }} onDragOver={(event) => event.preventDefault()} onDrop={(event) => { event.preventDefault(); selectFile(event.dataTransfer.files[0]); }}>
              <input ref={inputRef} type="file" accept="image/png,image/jpeg,image/webp,image/bmp,image/tiff" hidden onChange={(event) => selectFile(event.target.files?.[0])} />
              {preview ? <img src={preview} alt="Selected source" /> : <><span className="drop-mark">01</span><h2>Bring one image into focus.</h2><p>PNG, JPEG, WebP, BMP, or TIFF. The source is never modified.</p></>}
              <button className="primary-button" aria-label={file || nativePath ? "Choose another image" : "Choose image"} onClick={() => void chooseImage()}>{file || nativePath ? "Choose another image" : "Choose image"}</button>
            </div>
            {(file || nativePath) && <div className="file-meta"><span>{file?.name ?? nativePath.split(/[\\/]/).pop()}</span>{file && <span>{Math.round(file.size / 1024)} KB</span>}</div>}
          </div>
          <div className="detail-panel">
            <div className="panel-heading"><span className="overline">02 / Describe</span><span className={busy ? "status busy" : "status"} role="status" aria-live="polite">{notice}</span></div>
             <div className="controls"><label>Provider<select value={provider} onChange={(event) => selectProvider(event.target.value)}>{providerOptions.map((item) => <option value={item.id} key={item.id} disabled={!item.vision}>{item.name}{item.vision ? "" : " (unavailable)"}</option>)}</select></label>{provider === "chatgpt" && chatgptModels.length > 0
              ? <label>Model<select value={model} onChange={(event) => setModel(event.target.value)}>{chatgptModels.map((item) => <option value={item.id} key={item.id}>{item.displayName}</option>)}</select></label>
              : <label>Model<input value={model} onChange={(event) => setModel(event.target.value)} placeholder="vision" />{provider === "chatgpt" && <small className="muted">{chatgpt.configured ? "Loading models..." : "Sign in with ChatGPT in Settings to load models."}</small>}</label>}<label>Preset<select value={presetId} onChange={(event) => setPresetId(event.target.value)}>{presets.map((item) => <option value={item.id} key={item.id}>{item.name}</option>)}</select></label></div>
            <label className="description-field">Description<textarea value={description} onChange={(event) => setDescription(event.target.value)} placeholder="Your generated description will appear here. You can edit it before saving." /></label>
            <div className="action-row"><button className="primary-button" disabled={!nativePath || busy || !providerOptions.find((item) => item.id === provider)?.vision} onClick={() => void describe()}>{busy ? "Describing..." : "Describe image"}</button><button className="primary-button" disabled={!description.trim() || !nativePath || busy} onClick={() => void save()}>Save PNG copy</button></div>
          </div>
        </section>
      )}
    </main>
  );
}

export default App;
