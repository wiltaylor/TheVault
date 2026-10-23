// Test-only: the turns form (ai D23, agents G17) over a test's fake stream. A test scripts a
// turn's events with `stream(path, body, onEvent, signal)`, as POST /api/ai/chat once streamed
// them; `aiTurnStart` keeps each turn's body and `streamTurn` streams that turn through `stream`
// with the body it started with, so a test reads the body a turn sent from `stream`'s calls.

import { vi } from "vitest";
import type { AiTurnStarted, StreamEvent } from "../api";

type OnEvent = (e: StreamEvent) => void;
type Stream = (path: string, body: unknown, onEvent: OnEvent, signal?: AbortSignal) => Promise<void>;

export function turnsOver(stream: Stream) {
  const bodies = new Map<string, unknown>();
  let n = 0;
  const aiTurnStart = vi.fn(async (body: unknown): Promise<AiTurnStarted> => {
    const turn = `turn-${++n}`;
    bodies.set(turn, body);
    return { turn, started_ms: Date.now() };
  });
  const streamTurn = vi.fn((turn: string, _after: number, onEvent: OnEvent, signal?: AbortSignal) =>
    stream("/api/ai/chat", bodies.get(turn), onEvent, signal),
  );
  const aiTurnStop = vi.fn(async (_turn: string) => undefined);
  return { aiTurnStart, streamTurn, aiTurnStop };
}
