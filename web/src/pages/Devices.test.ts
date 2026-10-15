import { describe, expect, it } from "vitest";
import { BUILTIN_COMMANDS, commandChoices } from "./Devices";

describe("commandChoices", () => {
  it("offers the built-in commands when none are enabled", () => {
    expect(commandChoices([])).toEqual(BUILTIN_COMMANDS);
  });

  it("adds enabled commands the device declared beyond the built-in ones, once each", () => {
    expect(commandChoices(["send_sms", "ring_phone"])).toEqual([...BUILTIN_COMMANDS, "ring_phone"]);
  });
});
