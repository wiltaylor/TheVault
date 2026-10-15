// Runs in every page of the desktop app's window, before the page's own scripts. It adds the sync
// badge to the web client's pages, sends an unpaired app back to setup, offers a newer release
// when the update check found one (D27), and opens the capture dialog when `thevault --capture`,
// the hotkey or the tray asked for it. The web client itself is unchanged (D18).
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

  if (location.pathname.startsWith("/_desktop/")) return;

  const plural = (n, one, many) => `${n} ${n === 1 ? one : many}`;
  function describe(s) {
    const waiting = s.pending > 0 ? plural(s.pending, "change", "changes") + " waiting" : "";
    switch (s.phase) {
      case "starting":
        return "Starting sync…";
      case "syncing":
        return "Syncing…";
      case "synced":
        return waiting ? `Synced · ${waiting}` : "Synced";
      case "offline":
        return waiting ? `Offline · ${waiting}` : "Offline";
      case "update_required":
      case "revoked":
        return s.message;
      default:
        return `Sync failed: ${s.message ?? "unknown error"}`;
    }
  }

  function mount() {
    const badge = document.createElement("button");
    badge.id = "thevault-sync";
    badge.type = "button";
    badge.setAttribute("role", "status");
    badge.title = "Sync now";
    badge.style.cssText = [
      "position:fixed",
      "left:12px",
      "bottom:12px",
      "z-index:2147483000",
      "max-width:min(36rem,60vw)",
      "padding:4px 10px",
      "border:1px solid rgba(127,127,127,.35)",
      "border-radius:999px",
      "background:Canvas",
      "color:CanvasText",
      "font:12px/1.4 system-ui,sans-serif",
      "opacity:.85",
      "cursor:pointer",
      "text-align:left",
    ].join(";");
    badge.addEventListener("click", () => fetch("/_desktop/sync", { method: "POST" }));
    document.body.appendChild(badge);

    // The update prompt: shown while the app offers a newer release, until the user picks one of
    // its buttons. "Later" hides it until the app starts again.
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
    button("Download", "download").addEventListener("click", () => {
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
      text.textContent =
        update.source === "server"
          ? `TheVault ${update.version} is available from your server.`
          : `TheVault ${update.version} is available.`;
      show(true);
    }

    async function refresh() {
      try {
        const s = await (await fetch("/_desktop/status")).json();
        if (!s.paired) {
          location.replace("/_desktop/setup");
          return;
        }
        badge.dataset.phase = s.phase;
        badge.dataset.pending = String(s.pending);
        const alarm = s.phase === "update_required" || s.phase === "revoked" || s.phase === "error";
        badge.style.borderColor = alarm ? "#c62828" : "rgba(127,127,127,.35)";
        badge.textContent = describe(s);
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
