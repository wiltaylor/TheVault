// Runs in every page of the desktop app's window, before the page's own scripts. It sends an
// unpaired app back to setup, offers a newer release
// when the update check found one, or a restart once an update is downloaded (D27), and opens the capture dialog when `thevault --capture`,
// the hotkey or the tray asked for it. The web client itself is unchanged (D18): it shows the sync
// status itself, as the icon beside Archive, from the embedded server's `GET /api/app/sync`.
(() => {
  if (window.__thevaultPage || window.top !== window || location.protocol !== "vault:") return;
  window.__thevaultPage = true;

  const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

  // The web client opens its capture dialog on Ctrl+Shift+K once its shell is mounted.
  window.__thevaultCapture = async () => {
    const answer = await (await fetch("/_desktop/capture", { method: "POST" })).json();
    if (!answer.open) return;
    for (let i = 0; i < 200 && !document.querySelector(".capture-fab"); i++) await sleep(50);
    document.dispatchEvent(
      new KeyboardEvent("keydown", { key: "K", code: "KeyK", ctrlKey: true, shiftKey: true, bubbles: true, cancelable: true }),
    );
  };

  // The notice an AppImage leaves after installing itself in ~/Applications, on every page,
  // the setup page too, until the user answers it. Undo takes the menu entry away and keeps the
  // file.
  async function installedNotice() {
    let s;
    try {
      s = await (await fetch("/_desktop/status")).json();
    } catch {
      return;
    }
    const n = s.installed;
    if (!n || document.getElementById("thevault-installed")) return;
    const box = document.createElement("div");
    box.id = "thevault-installed";
    box.setAttribute("role", "status");
    box.style.cssText = [
      "position:fixed",
      "top:12px",
      "right:12px",
      "z-index:2147483001",
      "display:flex",
      "gap:8px",
      "align-items:center",
      "max-width:min(40rem,80vw)",
      "padding:8px 12px",
      "border:1px solid rgba(127,127,127,.35)",
      "border-radius:8px",
      "background:Canvas",
      "color:CanvasText",
      "font:13px/1.4 system-ui,sans-serif",
      "box-shadow:0 2px 8px rgba(0,0,0,.2)",
    ].join(";");
    const text = document.createElement("span");
    const menu = s.settings?.app_menu !== false;
    text.textContent =
      `TheVault was installed to ${n.path}${menu ? " and added to your apps" : ""}.` +
      (n.removed_original ? "" : ` You can delete ${n.original}.`);
    box.appendChild(text);
    const button = (label, action, path) => {
      const b = document.createElement("button");
      b.type = "button";
      b.textContent = label;
      b.dataset.action = action;
      b.style.cssText = "font:inherit;padding:2px 8px;cursor:pointer";
      b.addEventListener("click", () => {
        fetch(path, { method: "POST" });
        box.remove();
      });
      box.appendChild(b);
    };
    if (menu) button("Undo", "undo", "/_desktop/installed/undo");
    button("OK", "dismiss", "/_desktop/installed/dismiss");
    document.body.appendChild(box);
  }
  if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", installedNotice);
  else installedNotice();

  if (location.pathname.startsWith("/_desktop/")) return;

  function mount() {
    // The update prompt: shown while the app offers a newer release, until the user picks one of
    // its buttons. "Later" hides it until the app starts again. A downloaded update offers
    // "Restart to update"; an app installed from a deb or rpm is told to use its package manager.
    let later = null;
    const prompt = document.createElement("div");
    prompt.id = "thevault-update";
    prompt.setAttribute("role", "alert");
    prompt.style.cssText = [
      "display:none",
      "position:fixed",
      "right:12px",
      "bottom:12px",
      "z-index:2147483000",
      "gap:8px",
      "align-items:center",
      "padding:8px 12px",
      "border:1px solid rgba(127,127,127,.35)",
      "border-radius:8px",
      "background:Canvas",
      "color:CanvasText",
      "font:13px/1.4 system-ui,sans-serif",
      "box-shadow:0 2px 8px rgba(0,0,0,.2)",
    ].join(";");
    const text = document.createElement("span");
    const button = (label, action) => {
      const b = document.createElement("button");
      b.type = "button";
      b.textContent = label;
      b.dataset.action = action;
      b.style.cssText = "font:inherit;padding:2px 8px;cursor:pointer";
      prompt.appendChild(b);
      return b;
    };
    prompt.appendChild(text);
    const show = (on) => (prompt.style.display = on ? "flex" : "none");
    const restart = button("Restart to update", "restart");
    restart.addEventListener("click", () => {
      text.textContent = "Restarting…";
      fetch("/_desktop/update/restart", { method: "POST" });
    });
    const download = button("Download", "download");
    download.addEventListener("click", () => {
      fetch("/_desktop/update/open", { method: "POST" });
      later = prompt.dataset.version;
      show(false);
    });
    button("Skip this version", "skip").addEventListener("click", () => {
      fetch("/_desktop/update/skip", { method: "POST" });
      show(false);
    });
    button("Later", "later").addEventListener("click", () => {
      later = prompt.dataset.version;
      show(false);
    });
    document.body.appendChild(prompt);

    function offer(update) {
      if (!update || update.version === later) {
        show(false);
        return;
      }
      prompt.dataset.version = update.version;
      prompt.dataset.ready = String(Boolean(update.ready));
      restart.style.display = update.ready ? "" : "none";
      download.style.display = update.ready ? "none" : "";
      if (update.ready) {
        text.textContent = `TheVault ${update.version} is ready \u2014 restart to update.`;
      } else if (update.install === "package") {
        text.textContent = `TheVault ${update.version} is available. Update it with your package manager, or download it.`;
      } else {
        text.textContent =
          update.source === "server"
            ? `TheVault ${update.version} is available from your server.`
            : `TheVault ${update.version} is available.`;
      }
      show(true);
    }

    async function refresh() {
      try {
        const s = await (await fetch("/_desktop/status")).json();
        if (!s.paired) {
          location.replace("/_desktop/setup");
          return;
        }
        offer(s.update);
      } catch {}
    }
    refresh();
    setInterval(refresh, 1500);
    window.__thevaultCapture();
  }

  if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", mount);
  else mount();
})();
