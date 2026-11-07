// The Android app opens the bell panel when the user taps the summary of its notifications in
// the shade (TV-94). Its `VaultNotifications` bridge holds the request: the page takes it when
// it starts, for a tap that started the app, and again on each `vault:open-notifications` event,
// which the app sends for a tap while the page is up. In a browser there is no bridge.

interface NotificationsBridge {
  take(): boolean;
}

/** The window event the app sends after a tap on the summary. */
export const APP_OPEN_EVENT = "vault:open-notifications";

function bridge(): NotificationsBridge | undefined {
  return (window as unknown as { VaultNotifications?: NotificationsBridge }).VaultNotifications;
}

/**
 * Calls `open` for each request of the app to open the bell panel: one waiting now, and each one
 * announced after. Returns the function that stops listening.
 */
export function followAppOpen(open: () => void): () => void {
  const take = () => {
    let wanted = false;
    try {
      wanted = bridge()?.take() === true;
    } catch {
      // A bridge that fails opens nothing.
    }
    if (wanted) open();
  };
  take();
  window.addEventListener(APP_OPEN_EVENT, take);
  return () => window.removeEventListener(APP_OPEN_EVENT, take);
}
