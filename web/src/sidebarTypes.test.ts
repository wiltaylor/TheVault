import { describe, expect, it } from "vitest";
import { sortTypesForNav, typeNavLabel } from "./sidebarTypes";

const t = (name: string, plural: string, label = plural) => ({ name, plural, label });

describe("sidebar types", () => {
  it("label a type by its plural, falling back to its label and name", () => {
    expect(typeNavLabel(t("task", "Tasks", "Task"))).toBe("Tasks");
    expect(typeNavLabel(t("task", "", "Task"))).toBe("Task");
    expect(typeNavLabel(t("task", "", ""))).toBe("task");
  });

  it("sort by label ignoring case, then by name, without changing the input", () => {
    const input = [t("z", "zebras"), t("b", "Apples"), t("a", "apples"), t("m", "Mangoes")];
    expect(sortTypesForNav(input).map((x) => x.name)).toEqual(["a", "b", "m", "z"]);
    expect(input.map((x) => x.name)).toEqual(["z", "b", "a", "m"]);
  });
});
