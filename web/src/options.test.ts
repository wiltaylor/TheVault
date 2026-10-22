import { describe, expect, it } from "vitest";
import type { FieldDef } from "./api";
import { fieldText } from "./components/Render";
import { optionChoices, optionText } from "./options";

const calendar: FieldDef = {
  name: "calendar",
  label: "Calendar",
  description: null,
  kind: "select",
  required: false,
  computed: false,
  multiple: false,
  compound: false,
  options: ["primary", "work@group.calendar.google.com", "plain"],
  option_labels: ["Personal", "Work", ""],
};

describe("select option labels", () => {
  it("shows an option's label, else its value", () => {
    expect(optionText(calendar, "primary")).toBe("Personal");
    expect(optionText(calendar, "plain")).toBe("plain");
    expect(optionText(calendar, "gone")).toBe("gone");
    expect(optionText({ options: ["a"] }, "a")).toBe("a");
    expect(optionChoices(calendar)).toEqual([
      { value: "primary", text: "Personal" },
      { value: "work@group.calendar.google.com", text: "Work" },
      { value: "plain", text: "plain" },
    ]);
  });

  it("shows labels wherever a select's value is shown as text", () => {
    expect(fieldText(calendar, "work@group.calendar.google.com")).toBe("Work");
    expect(fieldText({ ...calendar, multiple: true }, ["primary", "plain"])).toBe("Personal, plain");
    expect(fieldText({ ...calendar, option_labels: undefined }, "primary")).toBe("primary");
    expect(fieldText(calendar, undefined)).toBe("");
  });
});
