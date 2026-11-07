import { afterEach, describe, expect, it, vi } from "vitest";
import { APP_OPEN_EVENT, followAppOpen } from "./appOpen";

type Win = { VaultNotifications?: unknown };

/** A bridge holding `pending` requests, taken one at a time as the app's is. */
function bridgeWith(pending: number) {
  let left = pending;
  const take = vi.fn(() => {
    if (left === 0) return false;
    left -= 1;
    return true;
  });
  (window as unknown as Win).VaultNotifications = { take };
  return { take, request: () => (left += 1) };
}

afterEach(() => {
  delete (window as unknown as Win).VaultNotifications;
});

describe("the app's request to open the bell panel", () => {
  it("opens nothing in a browser", () => {
    const open = vi.fn();
    const stop = followAppOpen(open);
    window.dispatchEvent(new Event(APP_OPEN_EVENT));
    stop();
    expect(open).not.toHaveBeenCalled();
  });

  it("opens the panel for a tap that started the app, once", () => {
    bridgeWith(1);
    const open = vi.fn();
    const stop = followAppOpen(open);
    expect(open).toHaveBeenCalledTimes(1);
    window.dispatchEvent(new Event(APP_OPEN_EVENT));
    expect(open).toHaveBeenCalledTimes(1);
    stop();
  });

  it("opens the panel for a tap while the page is up, until it stops listening", () => {
    const app = bridgeWith(0);
    const open = vi.fn();
    const stop = followAppOpen(open);
    expect(open).not.toHaveBeenCalled();
    app.request();
    window.dispatchEvent(new Event(APP_OPEN_EVENT));
    expect(open).toHaveBeenCalledTimes(1);
    stop();
    app.request();
    window.dispatchEvent(new Event(APP_OPEN_EVENT));
    expect(open).toHaveBeenCalledTimes(1);
  });

  it("opens nothing when the bridge fails", () => {
    (window as unknown as Win).VaultNotifications = {
      take: () => {
        throw new Error("gone");
      },
    };
    const open = vi.fn();
    followAppOpen(open)();
    expect(open).not.toHaveBeenCalled();
  });
});
