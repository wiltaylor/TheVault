import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it } from "vitest";
import type { FieldDef, Json } from "../api";
import { hostOf, isUrl, linkShape, linkText, linksText } from "../links";
import { FieldInput } from "./FieldInput";
import { pasteInto, rowsValue } from "./LinkList";
import { FieldValue, fieldText } from "./Render";

afterEach(cleanup);

function def(name: string, kind: string, more: Partial<FieldDef> = {}): FieldDef {
  return { name, label: name, description: null, kind, required: false, computed: false, multiple: false, compound: false, ...more };
}

/** A list whose items are a dict of `keys`, with no other entries unless `values` says so. */
function listOf(keys: FieldDef[], values: FieldDef["values"] = null): FieldDef {
  return def("links", "list", { label: "Links", items: "dict", item: def("links", "dict", { keys, values }) });
}

const LINKS = listOf([def("name", "string"), def("url", "url", { required: true })]);

describe("which lists are lists of links", () => {
  it("is a list of dicts of exactly a text name and a url, in either order, with no other entries", () => {
    expect(linkShape(LINKS)).toEqual({ nameRequired: false });
    expect(linkShape(listOf([def("url", "url"), def("name", "string", { required: true })]))).toEqual({ nameRequired: true });
  });

  it("is not a list with another key, other entries, the wrong kinds or names, or a dict on its own", () => {
    expect(linkShape(listOf([def("name", "string"), def("url", "url"), def("note", "string")]))).toBeUndefined();
    expect(linkShape(listOf([def("url", "url")]))).toBeUndefined();
    expect(linkShape(listOf([def("name", "string"), def("url", "url")], "any"))).toBeUndefined();
    expect(linkShape(listOf([def("name", "string"), def("url", "url")], def("other", "string")))).toBeUndefined();
    expect(linkShape(listOf([def("name", "string"), def("url", "string")]))).toBeUndefined();
    expect(linkShape(listOf([def("name", "url"), def("url", "url")]))).toBeUndefined();
    expect(linkShape(listOf([def("title", "string"), def("url", "url")]))).toBeUndefined();
    expect(linkShape(def("links", "dict", { keys: [def("name", "string"), def("url", "url")], values: null }))).toBeUndefined();
    expect(linkShape(def("links", "list", { items: "url", item: def("links", "url") }))).toBeUndefined();
  });

  it("takes URLs as a url field does", () => {
    expect(isUrl("https://example.com/docs")).toBe(true);
    expect(isUrl("mailto:wil@example.com")).toBe(true);
    expect(isUrl("example.com")).toBe(false);
    expect(isUrl("https://")).toBe(false);
    expect(isUrl("https://exa mple.com")).toBe(false);
    expect(isUrl("/relative/path")).toBe(false);
  });
});

describe("a list of links read-only", () => {
  it("shows each link by its name, else its URL's host and path", () => {
    expect(linkText({ name: "Docs", url: "https://example.com/docs" })).toBe("Docs");
    expect(linkText({ name: "", url: "https://www.example.com/blog/" })).toBe("example.com/blog");
    expect(linkText({ name: " ", url: "https://example.com/" })).toBe("example.com");
    expect(linkText({ name: "", url: "mailto:wil@example.com" })).toBe("mailto:wil@example.com");
    expect(hostOf("https://www.github.com/a")).toBe("github.com");
    const value: Json = [{ name: "Docs", url: "https://example.com/docs" }, { url: "https://example.org/blog" }];
    expect(fieldText(LINKS, value)).toBe("Docs, example.org/blog");
    expect(linksText([])).toBe("");
  });

  it("is links that open in a new tab, never a script", () => {
    const value: Json = [
      { name: "Docs", url: "https://example.com/docs" },
      { url: "https://example.org/blog" },
      { name: "Bad", url: "javascript:alert(1)" },
    ];
    const r = render(() => <FieldValue field={LINKS} value={value} />);
    const links = [...r.container.querySelectorAll("a")];
    expect(links.map((a) => [a.textContent, a.getAttribute("href"), a.target, a.rel])).toEqual([
      ["Docs", "https://example.com/docs", "_blank", "noopener noreferrer"],
      ["example.org/blog", "https://example.org/blog", "_blank", "noopener noreferrer"],
    ]);
    expect(r.container.textContent).toBe("Docs, example.org/blog, Bad");
  });
});

describe("rows of the links editor", () => {
  it("stores trimmed links, without a blank name, and leaves out rows with no URL or a bad one", () => {
    const shape = { nameRequired: false };
    expect(
      rowsValue(
        [
          { name: " Docs ", url: " https://example.com/docs " },
          { name: "", url: "https://example.org" },
          { name: "", url: "" },
          { name: "Typing", url: "https://" },
        ],
        shape,
      ),
    ).toEqual([{ name: "Docs", url: "https://example.com/docs" }, { url: "https://example.org" }]);
    expect(rowsValue([{ name: "", url: "" }], shape)).toBeUndefined();
    // A required name is always written, and a link without one waits for it.
    expect(rowsValue([{ name: "A", url: "https://a.example" }, { name: "", url: "https://b.example" }], { nameRequired: true })).toEqual([
      { name: "A", url: "https://a.example" },
    ]);
  });

  it("fills a pasted URL, and the name with its host when the name is blank", () => {
    expect(pasteInto({ name: "", url: "" }, " https://www.github.com/wiltaylor ")).toEqual({ name: "github.com", url: "https://www.github.com/wiltaylor" });
    expect(pasteInto({ name: "Mine", url: "" }, "https://example.com")).toEqual({ name: "Mine", url: "https://example.com" });
    expect(pasteInto({ name: "", url: "https://a.example" }, "https://example.com")).toBeNull();
    expect(pasteInto({ name: "", url: "" }, "just text")).toBeNull();
  });
});

/** A note's links in a signal, edited through FieldInput as the note page does. */
function mount(start: Json | undefined, field: FieldDef = LINKS) {
  const [value, setValue] = createSignal<Json | undefined>(start);
  const writes: (Json | undefined)[] = [];
  const r = render(() => (
    <FieldInput
      field={field}
      value={value()}
      onChange={(v) => {
        writes.push(v);
        setValue(v);
      }}
      label="Links"
      id="links"
    />
  ));
  const name = (n: number) => r.getByRole("textbox", { name: `Name of Links link ${n}` }) as HTMLInputElement;
  const url = (n: number) => r.getByRole("textbox", { name: `URL of Links link ${n}` }) as HTMLInputElement;
  const type = (el: HTMLInputElement, v: string) => {
    el.value = v;
    fireEvent.input(el);
  };
  const paste = (el: HTMLInputElement, text: string) =>
    fireEvent.paste(el, { clipboardData: { getData: (t: string) => (t === "text/plain" ? text : "") } });
  const rows = () => [...r.container.querySelectorAll<HTMLElement>(".link-row")].map((row) => [row.querySelector<HTMLInputElement>(".link-name-input")!.value, row.querySelector<HTMLInputElement>(".link-url-input")!.value]);
  return { ...r, value, setValue, writes, name, url, type, paste, rows };
}

describe("the links editor", () => {
  it("adds a link typed into Name and URL, without a name when it is blank, and an empty list is no value", () => {
    const m = mount(undefined);
    fireEvent.click(m.getByRole("button", { name: "Add link" }));
    m.type(m.name(1), "Docs");
    // Nothing is written until the row has a URL.
    expect(m.writes).toEqual([]);
    m.type(m.url(1), "https://example.com/docs");
    expect(m.value()).toEqual([{ name: "Docs", url: "https://example.com/docs" }]);
    m.type(m.name(1), "");
    expect(m.value()).toEqual([{ url: "https://example.com/docs" }]);
    fireEvent.click(m.getByRole("button", { name: "Remove example.com/docs" }));
    expect(m.value()).toBeUndefined();
    expect(m.rows()).toEqual([]);
  });

  it("fills the URL and the name from a URL pasted into a new row", () => {
    const m = mount([{ name: "Docs", url: "https://example.com/docs" }]);
    fireEvent.click(m.getByRole("button", { name: "Add link" }));
    m.paste(m.name(2), "https://github.com/wiltaylor/thevault");
    expect(m.rows()[1]).toEqual(["github.com", "https://github.com/wiltaylor/thevault"]);
    expect(m.value()).toEqual([
      { name: "Docs", url: "https://example.com/docs" },
      { name: "github.com", url: "https://github.com/wiltaylor/thevault" },
    ]);
    // Text that is not a URL pastes as usual.
    fireEvent.click(m.getByRole("button", { name: "Add link" }));
    m.paste(m.name(3), "Just a name");
    expect(m.rows()[2]).toEqual(["", ""]);
  });

  it("moves links up and down and removes one, keeping each name with its URL", () => {
    const m = mount([
      { name: "A", url: "https://a.example" },
      { name: "B", url: "https://b.example" },
      { url: "https://c.example/x" },
    ]);
    expect((m.getByRole("button", { name: "Move A up" }) as HTMLButtonElement).disabled).toBe(true);
    fireEvent.click(m.getByRole("button", { name: "Move A down" }));
    expect(m.value()).toEqual([{ name: "B", url: "https://b.example" }, { name: "A", url: "https://a.example" }, { url: "https://c.example/x" }]);
    fireEvent.click(m.getByRole("button", { name: "Move c.example/x up" }));
    expect(m.rows()).toEqual([
      ["B", "https://b.example"],
      ["", "https://c.example/x"],
      ["A", "https://a.example"],
    ]);
    fireEvent.click(m.getByRole("button", { name: "Remove B" }));
    expect(m.value()).toEqual([{ url: "https://c.example/x" }, { name: "A", url: "https://a.example" }]);
    // Each row opens its link in a new tab.
    const open = m.getByRole("link", { name: "Open A" }) as HTMLAnchorElement;
    expect([open.getAttribute("href"), open.target, open.rel]).toEqual(["https://a.example", "_blank", "noopener noreferrer"]);
  });

  it("says when a URL is not one, once its box is left, and stores the other links meanwhile", () => {
    const m = mount([{ name: "A", url: "https://a.example" }]);
    fireEvent.click(m.getByRole("button", { name: "Add link" }));
    m.type(m.name(2), "Site");
    m.type(m.url(2), "example.com");
    expect(m.queryByRole("alert")).toBeNull();
    fireEvent.blur(m.url(2));
    expect(m.getByRole("alert").textContent).toBe("“example.com” is not an absolute URL, such as https://example.com.");
    expect(m.url(2).getAttribute("aria-invalid")).toBe("true");
    expect(m.value()).toEqual([{ name: "A", url: "https://a.example" }]);
    m.type(m.url(2), "https://example.com");
    expect(m.queryByRole("alert")).toBeNull();
    expect(m.value()).toEqual([
      { name: "A", url: "https://a.example" },
      { name: "Site", url: "https://example.com" },
    ]);
  });

  it("starts again from a value written elsewhere, and keeps its rows for its own writes", () => {
    const m = mount([{ name: "A", url: "https://a.example" }]);
    fireEvent.click(m.getByRole("button", { name: "Add link" }));
    m.type(m.name(2), "Half done");
    // Its own write leaves the half-done row alone.
    m.type(m.name(1), "Alpha");
    expect(m.rows()).toEqual([
      ["Alpha", "https://a.example"],
      ["Half done", ""],
    ]);
    m.setValue([{ name: "Z", url: "https://z.example" }]);
    expect(m.rows()).toEqual([["Z", "https://z.example"]]);
  });

  it("waits for a required name, and asks for it", () => {
    const m = mount(undefined, listOf([def("name", "string", { required: true }), def("url", "url", { required: true })]));
    fireEvent.click(m.getByRole("button", { name: "Add link" }));
    m.type(m.url(1), "https://example.com");
    fireEvent.blur(m.url(1));
    expect(m.value()).toBeUndefined();
    expect(m.getByRole("alert").textContent).toBe("Enter the link's name.");
    m.type(m.name(1), "Ex");
    expect(m.value()).toEqual([{ name: "Ex", url: "https://example.com" }]);
  });
});
