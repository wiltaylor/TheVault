import { describe, expect, it } from "vitest";
import { BUILTIN_COMMANDS, commandChoices, declares } from "./Devices";

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
