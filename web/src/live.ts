// The server's live event stream (D4): `/api/events` tells an open client what changed as it
// changes, so the bell and open notes need not wait for a poll. While the stream is down, the
// caller polls as it did before the stream existed.

/** How long to wait before opening the stream again once the browser has given up on it. */
export const RETRY_MS = 10_000;

/** Every event the server streams. */
export const LIVE_EVENTS = [
  "notification.created",
  "notification.updated",
  "notification.read",
  "notification.dismissed",
  "notification.cleared",
  "note.changed",
  "device.command",
  "device.settings",
  "ai_turn",
] as const;

export type LiveEventName = (typeof LIVE_EVENTS)[number];

export interface LiveHandlers {
  /** An event came, with its parsed data. */
  event(name: LiveEventName, data: unknown): void;
  /** The stream opened. `again` is true when it had been down: events may have been missed. */
  open(again: boolean): void;
  /** The stream failed or closed. Called once per failure, until it opens again. */
  down(): void;
}

/** The parts of `EventSource` used here, so a test can stand one in. */
export interface Source {
  readonly readyState: number;
  addEventListener(type: string, listener: (e: MessageEvent) => void): void;
  close(): void;
}

const CLOSED = 2;

function browserSource(url: string): Source | null {
  if (typeof EventSource === "undefined") return null;
  return new EventSource(url, { withCredentials: true });
}

/**
 * Keep a connection to `/api/events` until the returned function is called. The browser retries a
 * dropped stream itself; one it gives up on (an error status, such as a 401) is opened again after
 * [`RETRY_MS`].
 */
export function connectLive(handlers: LiveHandlers, open: (url: string) => Source | null = browserSource): () => void {
  let source: Source | null = null;
  let retry: ReturnType<typeof setTimeout> | undefined;
  let stopped = false;
  let up = false;
  let wasDown = false;

  const fail = () => {
    if (up || !wasDown) handlers.down();
    up = false;
    wasDown = true;
  };

  const connect = () => {
    retry = undefined;
    if (stopped) return;
    const s = open("/api/events");
    source = s;
    if (!s) {
      fail();
      return;
    }
    s.addEventListener("open", () => {
      if (stopped || source !== s) return;
      up = true;
      handlers.open(wasDown);
    });
    s.addEventListener("error", () => {
      if (stopped || source !== s) return;
      fail();
      if (s.readyState === CLOSED) {
        s.close();
        retry = setTimeout(connect, RETRY_MS);
      }
    });
    for (const name of LIVE_EVENTS) {
      s.addEventListener(name, (e) => {
        if (stopped || source !== s) return;
        let data: unknown = null;
        try {
          data = JSON.parse(e.data as string);
        } catch {
          return;
        }
        handlers.event(name, data);
      });
    }
  };

  connect();
  return () => {
    stopped = true;
    clearTimeout(retry);
    source?.close();
  };
}
