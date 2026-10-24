import { describe, expect, it } from "vitest";
import type { AgentThread } from "../api";
import { tokens } from "./ChatPanel";
import { turnsFromThread } from "./store";

const msg = (role: "user" | "assistant", text: string) => ({ role, content: [{ type: "text", text }] });

const thread = (messages: ReturnType<typeof msg>[], upto: number | null): AgentThread =>
  ({
    id: "t",
    agent: "assistant",
    version: "v",
    title: "T",
    messages,
    compaction: upto === null ? null : { summary: "The gist.", upto, at_ms: 1, before: 10, reason: "auto" },
  }) as unknown as AgentThread;

describe("a compacted thread", () => {
  const four = [msg("user", "Q1"), msg("assistant", "A1"), msg("user", "Q2"), msg("assistant", "A2")];

  it("marks the summary at the end of the answer before the first message the model still reads", () => {
    const turns = turnsFromThread(thread(four, 2));
    expect(turns.map((t) => t.role)).toEqual(["user", "assistant", "user", "assistant"]);
    expect(turns[1]!.parts).toEqual([
      { kind: "text", text: "A1" },
      { kind: "compaction", summary: "The gist.", reason: "auto" },
    ]);
    expect(turns[3]!.parts.some((p) => p.kind === "compaction")).toBe(false);
  });

  it("marks it after the last answer when the whole thread was compacted, and nowhere when it was not", () => {
    const all = turnsFromThread(thread(four, 4));
    expect(all[3]!.parts.at(-1)).toEqual({ kind: "compaction", summary: "The gist.", reason: "auto" });
    const none = turnsFromThread(thread(four, null));
    expect(none.flatMap((t) => t.parts).some((p) => p.kind === "compaction")).toBe(false);
  });
});

describe("tokens", () => {
  it("reads as people count them", () => {
    expect(tokens(950)).toBe("950");
    expect(tokens(1230)).toBe("1.2k");
    expect(tokens(42_000)).toBe("42k");
    expect(tokens(1_234_567)).toBe("1.2M");
  });
});
