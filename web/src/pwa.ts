// Registers the service worker, which caches the app shell only. Notes are never cached, so there
// is no offline mode (D18).

export function registerServiceWorker() {
  if (!import.meta.env.PROD || !("serviceWorker" in navigator)) return;
  window.addEventListener("load", () => {
    navigator.serviceWorker.register("/sw.js").catch((e: unknown) => {
      console.warn("service worker registration failed", e);
    });
  });
}
