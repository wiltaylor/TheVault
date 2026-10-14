import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createAutosave } from "./autosave";

describe("autosave", () => {
  beforeEach(() => void vi.useFakeTimers());
  afterEach(() => void vi.useRealTimers());

  it("saves once, a delay after the last of several changes", async () => {
    const save = vi.fn(async () => {});
    const a = createAutosave({ delay: 1500, save });
    a.schedule();
    await vi.advanceTimersByTimeAsync(1000);
    a.schedule();
    await vi.advanceTimersByTimeAsync(1000);
    expect(save).not.toHaveBeenCalled();
    expect(a.state()).toBe("dirty");
    await vi.advanceTimersByTimeAsync(600);
    expect(save).toHaveBeenCalledTimes(1);
    expect(a.state()).toBe("saved");
    expect(a.dirty()).toBe(false);
  });

  it("flushes at once, and does nothing when there is nothing to save", async () => {
    const save = vi.fn(async () => {});
    const a = createAutosave({ delay: 1500, save });
    await a.flush();
    expect(save).not.toHaveBeenCalled();
    a.schedule();
    await a.flush();
    expect(save).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(5000);
    expect(save).toHaveBeenCalledTimes(1);
  });

  it("never overlaps saves: a change during a save is saved after it", async () => {
    let release: () => void = () => {};
    const save = vi.fn(() => new Promise<void>((r) => (release = r)));
    const states: string[] = [];
    const a = createAutosave({ delay: 100, save, onState: (s) => states.push(s) });
    a.schedule();
    const first = a.flush();
    expect(save).toHaveBeenCalledTimes(1);
    a.schedule();
    void a.flush();
    expect(save).toHaveBeenCalledTimes(1);
    release();
    await vi.advanceTimersByTimeAsync(0);
    expect(save).toHaveBeenCalledTimes(2);
    release();
    await first;
    expect(states.at(-1)).toBe("saved");
    expect(a.dirty()).toBe(false);
  });

  it("keeps the change unsaved when a save fails, and cancel drops it", async () => {
    const save = vi.fn(async () => {
      throw new Error("nope");
    });
    const a = createAutosave({ delay: 100, save });
    a.schedule();
    await a.flush();
    expect(a.state()).toBe("error");
    expect(a.dirty()).toBe(true);
    a.cancel();
    expect(a.dirty()).toBe(false);
    expect(a.state()).toBe("idle");
  });
});
