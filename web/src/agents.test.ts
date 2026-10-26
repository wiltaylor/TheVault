// The agent helpers (agents G3, G5): the name a typed agent name becomes, the tool checklist's
// groups, and the command palette's New agent and Open agent actions.

import { describe, expect, it } from "vitest";
import { agentNameFrom, scriptsBlock, toolGroups } from "./agents";
import type { AgentSummary } from "./api";
import { agentActions } from "./palette/actions";

describe("agentNameFrom", () => {
  it("makes a vault name of what was typed", () => {
    expect(agentNameFrom("Inbox triage")).toBe("inbox_triage");
    expect(agentNameFrom("  Café -- Digest! ")).toBe("cafe_digest");
    expect(agentNameFrom("daily_digest")).toBe("daily_digest");
    expect(agentNameFrom("2nd brain")).toBe("agent_2nd_brain");
    expect(agentNameFrom("!!!")).toBe("");
    expect(agentNameFrom("a".repeat(80))).toHaveLength(64);
  });
});

describe("toolGroups", () => {
  it("groups the server's tools in order and puts unknown ones in Other", () => {
    const groups = toolGroups(["web_search", "search_notes", "propose_edit", "summon_cat"]);
    expect(groups.map((g) => g.group)).toEqual(["Notes (read)", "Notes (changes)", "Internet", "Other"]);
    expect(groups[0]!.tools).toEqual([{ name: "search_notes", label: "Search notes" }]);
    expect(groups.at(-1)!.tools).toEqual([{ name: "summon_cat", label: "summon_cat" }]);
  });
});

describe("scriptsBlock", () => {
  it("keeps only the switches that are on, and none at all when none is", () => {
    expect(scriptsBlock(undefined)).toBeUndefined();
    expect(scriptsBlock({ run_code: false, run_saved: false })).toBeUndefined();
    expect(scriptsBlock({ run_code: false, run_saved: true })).toEqual({ run_saved: true });
    expect(scriptsBlock({ run_code: true, run_saved: true })).toEqual({ run_code: true, run_saved: true });
  });

  it("groups the saved script and task tools with the scripts", () => {
    const groups = toolGroups(["run_saved_script", "run_task", "list_tasks", "list_scripts"]);
    expect(groups.map((g) => g.group)).toEqual(["Scripts"]);
  });
});

describe("the palette's agent actions", () => {
  const list: AgentSummary[] = [
    { name: "assistant", label: "Assistant", icon: "sparkles", built_in: true },
    { name: "inbox_triage", label: "Inbox triage", built_in: false },
  ];

  it("offers New agent, Open browser box, Open agent for each, and New skill", () => {
    const actions = agentActions(list, false);
    expect(actions.map((a) => a.label)).toEqual([
      "New agent",
      "Open browser box",
      "Open agent Assistant",
      "Open agent Inbox triage",
      "New skill",
    ]);
    expect(actions[1]!.href).toBe("/browser-box");
    expect(actions[3]!.href).toBe("/agents/inbox_triage");
    expect(actions[2]!.icon).toEqual({ type: "sparkles" });
    expect(actions[3]!.icon).toEqual({ type: "bot" });
    expect(typeof actions[0]!.run).toBe("function");
  });

  it("leaves out the browser box and New skill in the apps, and keeps New agent", () => {
    expect(agentActions(list, true).map((a) => a.label)).toEqual(["New agent", "Open agent Assistant", "Open agent Inbox triage"]);
  });
});
