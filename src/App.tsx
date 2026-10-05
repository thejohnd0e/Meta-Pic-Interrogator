import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import "./App.css";

function App() {
  const [health, setHealth] = useState("checking");

  useEffect(() => {
    invoke<string>("health")
      .then(setHealth)
      .catch(() => setHealth("unavailable"));
  }, []);

  return (
    <main className="shell">
      <header className="masthead">
        <div>
          <p className="eyebrow">MetaPic Interrogator</p>
          <h1>Describe one image. Keep its provenance.</h1>
          <p className="lede">
            A focused workspace for generating, editing, and saving image
            descriptions as clean PNG copies.
          </p>
        </div>
        <span className={`status status-${health}`} role="status">
          Backend {health}
        </span>
      </header>

      <section className="drop-zone" aria-label="Image import area">
        <p className="drop-kicker">Ready for an image</p>
        <h2>Drop a supported image here</h2>
        <p>PNG, JPEG, WebP, BMP, or TIFF. The original is never modified.</p>
        <button type="button" disabled>
          Choose image
        </button>
      </section>
    </main>
  );
}

export default App;
