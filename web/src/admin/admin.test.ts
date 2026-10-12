import { describe, expect, it } from "vitest";
import type { Task } from "../api";
import { emptyTask, isoDuration, lines, taskFromApi, taskProblems, taskWcl } from "./taskModel";

describe("task model", () => {
  it("writes only the items that differ from their defaults", () => {
    const m = emptyTask();
    m.schedule = "0 7 * * 1-5";
    m.script = "digest";
    expect(taskWcl(m)).toBe(`schedule = "0 7 * * 1-5"\nscript = "digest"\n`);
  });

  it("writes every item it was given", () => {
    const m = emptyTask();
    Object.assign(m, {
      label: "Daily digest",
      description: "Posts the digest",
      schedule: "0 7 * * 1-5",
      timezone: "Australia/Melbourne",
      script: "digest",
      function: "send",
      allow_domains: ["hooks.example.com"],
      secrets: ["digest_webhook"],
      retries: 0,
      retry_delay: "PT5M",
      timeout: "PT2M",
      paused: true,
    });
    expect(taskWcl(m)).toBe(`label = "Daily digest"
description = "Posts the digest"
schedule = "0 7 * * 1-5"
timezone = "Australia/Melbourne"
script = "digest"
function = "send"
allow_domains = ["hooks.example.com"]
secrets = ["digest_webhook"]
retries = 0
retry_delay = "PT5M"
timeout = "PT2M"
paused = true
`);
  });

  it("reads a task the API describes back into the same file", () => {
    const task: Task = {
      name: "digest",
      next_run: null,
      retry: null,
      running: false,
      failing: false,
      latest_run: null,
      label: "digest",
      description: null,
      schedule: "*/5 * * * *",
      timezone: "UTC",
      script: "digest",
      function: "run",
      allow_domains: [],
      secrets: ["key"],
      retries: 3,
      retry_delay_ms: 90_000,
      timeout_ms: 30_000,
      paused: false,
    };
    expect(taskWcl(taskFromApi(task))).toBe(
      `schedule = "*/5 * * * *"\nscript = "digest"\nsecrets = ["key"]\nretry_delay = "PT90S"\n`,
    );
  });

  it("writes durations as the shortest exact unit", () => {
    expect(isoDuration(7_200_000)).toBe("PT2H");
    expect(isoDuration(60_000)).toBe("PT1M");
    expect(isoDuration(1_500)).toBe("PT1.5S");
  });

  it("reports a bad schedule, a missing script and negative retries", () => {
    const m = emptyTask();
    m.schedule = "0 7 * *";
    m.retries = -1;
    expect(taskProblems(m)).toEqual([
      "The schedule needs five cron fields: minute, hour, day of month, month, day of week.",
      "Choose the script the task runs.",
      "Retries must be a whole number, 0 or more.",
    ]);
  });

  it("splits one entry per line", () => {
    expect(lines(" a \n\nb\n")).toEqual(["a", "b"]);
  });
});

describe("wclTitle", () => {
  it("reads the title of a note's WCL", async () => {
    const { wclTitle } = await import("./format");
    expect(wclTitle(`title   = "Say \\"hi\\""\ncreated = "2026-10-12T09:15:00Z"\n`)).toBe('Say "hi"');
    expect(wclTitle(`created = "2026-10-12T09:15:00Z"\n`)).toBeNull();
  });
});
