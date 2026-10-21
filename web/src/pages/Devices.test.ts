import { describe, expect, it } from "vitest";
import { BUILTIN_COMMANDS, commandChoices, declares, freshStartText } from "./Devices";

describe("freshStartText", () => {
  it("says the device joined another vault, where the old local vault went and for how long", () => {
    const text = freshStartText(
      {
        at_ms: 1_000,
        previous: "/data/user/0/dev.wiltaylor.thevault/vault.prev-1000",
        from_vault: "a",
        to_vault: "b",
      },
      () => "21 Oct 2026",
    );
    expect(text).toContain("joined a different vault on 21 Oct 2026");
    expect(text).toContain("moved to vault.prev-1000 on the device");
    expect(text).toContain("removed 7 days after the move");
  });
});

describe("commandChoices", () => {
  it("offers the built-in commands when none are enabled", () => {
    expect(commandChoices([])).toEqual(BUILTIN_COMMANDS);
    expect(BUILTIN_COMMANDS).toContain("import_history");
  });

  it("adds enabled commands the device declared beyond the built-in ones, once each", () => {
    expect(commandChoices(["send_sms", "ring_phone"])).toEqual([...BUILTIN_COMMANDS, "ring_phone"]);
  });
});

describe("declares", () => {
  it("is true only for a command the platform's file declares", () => {
    const files = [
      { platform: "android" as const, commands: [{ name: "import_history", script: "device_import", function: "import_history" }] },
      { platform: "linux" as const, error: "bad" },
    ];
    expect(declares(files, "android", "import_history")).toBe(true);
    expect(declares(files, "android", "send_sms")).toBe(false);
    expect(declares(files, "linux", "import_history")).toBe(false);
    expect(declares(undefined, "android", "import_history")).toBe(false);
  });
});
