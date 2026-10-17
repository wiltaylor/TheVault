import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it } from "vitest";
import { TypeWarnings } from "./TypeEditor";

afterEach(cleanup);

describe("TypeWarnings", () => {
  it("lists what this version ignored in the type file, and says the type still works", () => {
    const m = render(() => (
      <TypeWarnings
        warnings={[
          { at: "type.shelf", message: "is not an attribute of types in this version; it is ignored", severity: "warning" },
          { at: "field size.unit", message: "is not an attribute of int fields in this version; it is ignored", severity: "warning" },
        ]}
      />
    ));
    const section = m.getByRole("region", { name: "Warnings" });
    expect(section.textContent).toContain("The type still works");
    expect([...section.querySelectorAll("li")].map((li) => li.textContent)).toEqual([
      "type.shelf: is not an attribute of types in this version; it is ignored",
      "field size.unit: is not an attribute of int fields in this version; it is ignored",
    ]);
  });

  it("shows nothing when the type file loaded whole", () => {
    const m = render(() => <TypeWarnings warnings={[]} />);
    expect(m.queryByRole("region", { name: "Warnings" })).toBeNull();
  });
});
