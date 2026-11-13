const $ = (id) => document.getElementById(id);
const form = $("pair");

function mode() {
  return form.elements.with.value;
}
function showMode() {
  $("password-row").hidden = mode() !== "password";
  $("code-row").hidden = mode() !== "code";
}
for (const radio of form.elements.with) radio.addEventListener("change", showMode);

// A QR code's payload pasted as the code fills in the server too.
$("code").addEventListener("input", () => {
  try {
    const payload = JSON.parse($("code").value);
    if (payload && typeof payload.code === "string") {
      $("code").value = payload.code;
      if (typeof payload.server_url === "string") $("server").value = payload.server_url;
    }
  } catch {}
});

fetch("/_desktop/status")
  .then((r) => r.json())
  .then((s) => {
    if (!$("name").value) $("name").value = s.default_name || "";
  })
  .catch(() => {});

// The first sync's progress, from the status's `progress` (ticket 144).
const MB = 1024 * 1024;
const mb = (n) => (n / MB).toFixed(1);
function left(secs) {
  if (secs < 60) return `about ${Math.max(1, secs)} s left`;
  const min = Math.round(secs / 60);
  if (min < 60) return `about ${min} min left`;
  return `about ${Math.floor(min / 60)} h ${min % 60} min left`;
}
let shown = 0;
function showProgress(p) {
  if (!p || !p.running) return;
  const pulled = p.stage === "pulling" || p.stage === "fetching";
  $("sync-notes").textContent = pulled ? `${p.notes_done} / ${p.notes_total}` : "…";
  $("sync-files").textContent = pulled
    ? `${p.files_done} / ${p.files_total} (${mb(p.bytes_done)} / ${mb(p.bytes_total)} MB)`
    : "…";
  $("sync-eta").textContent = p.eta_secs == null ? "Estimating…" : p.eta_secs === 0 ? "Finishing…" : left(p.eta_secs);
  // The bar: the share of the round's time gone once there is an estimate, else the share of
  // the work done. It never moves back.
  let share;
  if (p.eta_secs != null && p.elapsed_ms > 0) share = p.elapsed_ms / (p.elapsed_ms + p.eta_secs * 1000);
  else {
    const parts = [];
    if (p.notes_total > 0) parts.push(p.notes_done / p.notes_total);
    if (p.bytes_total > 0) parts.push(p.bytes_done / p.bytes_total);
    share = parts.length ? parts.reduce((a, b) => a + b, 0) / parts.length : 0;
  }
  shown = Math.max(shown, Math.min(1, share));
  $("sync-bar").value = shown;
}

const FAILED = ["update_required", "revoked", "error", "offline"];
// The failed round the user asked to retry: its error stays hidden.
let retried = null;
$("retry").addEventListener("click", async () => {
  retried = lastFailure;
  $("sync-error").textContent = "";
  $("retry").hidden = true;
  await fetch("/_desktop/sync", { method: "POST" }).catch(() => {});
});
let lastFailure = null;

async function waitForFirstSync() {
  $("progress").textContent = "";
  $("pair").hidden = true;
  $("first-sync").hidden = false;
  for (;;) {
    let s;
    try {
      s = await (await fetch("/_desktop/status")).json();
    } catch {
      await new Promise((r) => setTimeout(r, 500));
      continue;
    }
    if (s.last_sync_ms) break;
    showProgress(s.progress);
    if (FAILED.includes(s.phase) && s.last_attempt_ms !== retried) {
      lastFailure = s.last_attempt_ms;
      $("sync-error").textContent = s.message || "Sync failed.";
      $("retry").hidden = false;
    }
    await new Promise((r) => setTimeout(r, 500));
  }
  $("sync-bar").value = 1;
  location.replace("/");
}

form.addEventListener("submit", async (e) => {
  e.preventDefault();
  $("error").textContent = "";
  const body = { server_url: $("server").value, name: $("name").value };
  if (mode() === "password") body.password = $("password").value;
  else body.code = $("code").value;
  $("submit").disabled = true;
  $("progress").textContent = "Pairing…";
  try {
    const res = await fetch("/_desktop/pair", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(body),
    });
    const answer = await res.json().catch(() => ({}));
    if (!res.ok) {
      $("error").textContent = answer.message || `Pairing failed (${res.status}).`;
      $("progress").textContent = "";
      $("submit").disabled = false;
      return;
    }
    await waitForFirstSync();
  } catch (err) {
    $("error").textContent = String(err);
    $("progress").textContent = "";
    $("submit").disabled = false;
  }
});
