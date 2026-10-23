// TheVault service worker. It caches the app shell (the page, its scripts and styles, the
// manifest and the icons) so the app opens fast. It never caches /api: notes are always read from
// the server, and there is no offline mode (D18).

const CACHE = "thevault-shell-v2";
const SHELL = ["/", "/manifest.webmanifest", "/favicon.ico", "/icon-32.png", "/icon-192.png"];

self.addEventListener("install", (event) => {
  event.waitUntil(
    caches
      .open(CACHE)
      .then((cache) => cache.addAll(SHELL))
      .then(() => self.skipWaiting()),
  );
});

self.addEventListener("activate", (event) => {
  event.waitUntil(
    caches
      .keys()
      .then((keys) => Promise.all(keys.filter((k) => k !== CACHE).map((k) => caches.delete(k))))
      .then(() => self.clients.claim()),
  );
});

async function networkFirst(request, fallbackUrl) {
  const cache = await caches.open(CACHE);
  try {
    const response = await fetch(request);
    if (response.ok) await cache.put(fallbackUrl ?? request, response.clone());
    return response;
  } catch (error) {
    const cached = await cache.match(fallbackUrl ?? request);
    if (cached) return cached;
    throw error;
  }
}

async function cacheFirst(request) {
  const cache = await caches.open(CACHE);
  const cached = await cache.match(request);
  if (cached) return cached;
  const response = await fetch(request);
  if (response.ok) await cache.put(request, response.clone());
  return response;
}

self.addEventListener("fetch", (event) => {
  const request = event.request;
  const url = new URL(request.url);
  if (request.method !== "GET" || url.origin !== self.location.origin) return;
  if (url.pathname === "/api" || url.pathname.startsWith("/api/")) return;
  // The browser box's desktop (noVNC through the server) is not the app: its page must never be
  // stored as the shell.
  if (url.pathname === "/browser" || url.pathname.startsWith("/browser/")) return;

  if (request.mode === "navigate") {
    // Every app route is the same shell page.
    event.respondWith(networkFirst(request, "/"));
  } else if (url.pathname.startsWith("/assets/")) {
    // Vite names these by content hash, so a cached copy never goes stale.
    event.respondWith(cacheFirst(request));
  } else if (SHELL.includes(url.pathname)) {
    event.respondWith(networkFirst(request));
  }
});
