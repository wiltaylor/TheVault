import { readFileSync } from "node:fs";
import { render, waitFor } from "@solidjs/testing-library";
import { describe, expect, it } from "vitest";
import { TypeIcon, iconNames, kebabName, pascalName } from "./Icon";

describe("type icons", () => {
  it("are the names the schema crate accepts for an admonition's icon, no more and no fewer", async () => {
    // crates/schema/src/icons.txt is written from this list; refresh it when lucide changes.
    const rust = readFileSync(`${process.cwd()}/../crates/schema/src/icons.txt`, "utf8").trim().split("\n");
    expect(rust).toEqual(await iconNames());
  });

  it("turns a vault icon name into Lucide's export name", () => {
    expect(pascalName("file-text")).toBe("FileText");
    expect(pascalName("check-square")).toBe("CheckSquare");
    expect(pascalName("user")).toBe("User");
  });

  it("draws the named icon as SVG shapes", async () => {
    const { container } = render(() => <TypeIcon name="user" />);
    await waitFor(() => expect(container.querySelectorAll("svg circle").length).toBe(1));
    const circle = container.querySelector("svg circle")!;
    expect(circle.namespaceURI).toBe("http://www.w3.org/2000/svg");
  });

  it("draws a default icon for an unknown name or no name", async () => {
    for (const name of ["no-such-icon", null]) {
      const { container, unmount } = render(() => <TypeIcon name={name} />);
      await waitFor(() => expect(container.querySelectorAll("svg path").length).toBeGreaterThan(0));
      unmount();
    }
  });
});

describe("icon names", () => {
  it("gives every Lucide icon a name that leads back to it", async () => {
    const { icons } = await import("lucide");
    const drawings = icons as unknown as Record<string, unknown>;
    const keys = Object.keys(drawings);
    const names = keys.map(kebabName);
    // Two export names share a vault name only when they are aliases with the same drawing,
    // such as ArrowDownAZ and ArrowDownAz.
    const first = new Map<string, string>();
    for (const key of keys) {
      const name = kebabName(key);
      const seen = first.get(name);
      if (seen) expect(JSON.stringify(drawings[key]), `${seen} and ${key}`).toBe(JSON.stringify(drawings[seen]));
      else first.set(name, key);
    }
    expect(names).toContain("file-text");
    expect(names).toContain("calendar-days");
    expect(names).toContain("a-arrow-down");
  });

  it("offers every icon by name, sorted", async () => {
    const names = await iconNames();
    expect(names.length).toBeGreaterThan(1000);
    expect([...names].sort()).toEqual(names);
  });

  it("draws an icon chosen by a name with a capital run, such as a-arrow-down", async () => {
    const { container } = render(() => <TypeIcon name="a-arrow-down" />);
    await waitFor(() => expect(container.querySelectorAll("svg path").length).toBeGreaterThan(0));
    // Not the default file icon: that one has a path, a-arrow-down has several.
    const { icons } = await import("lucide");
    const expected = (icons as unknown as Record<string, unknown[]>).AArrowDown!.length;
    expect(container.querySelector("svg")!.children.length).toBe(expected);
  });
});
