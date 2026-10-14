import { fireEvent, render, waitFor } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { describe, expect, it, vi } from "vitest";
import type { Term } from "../api";
import { ChipPicker, type ChipPickerProps, termSuggestions, typeSuggestions } from "./ChipPicker";

const FRUIT = ["Apple", "Banana", "Cherry"].map((f) => ({ value: f.toLowerCase(), text: f }));
const fruit = (q: string) => FRUIT.filter((f) => f.text.toLowerCase().includes(q.trim().toLowerCase()));

function mount(start: string[], extra: Partial<ChipPickerProps> = {}) {
  const [values, setValues] = createSignal(start);
  const onReject = vi.fn();
  const r = render(() => (
    <ChipPicker label="Fruit" values={values()} suggest={fruit} onChange={setValues} onReject={onReject} {...extra} />
  ));
  const plus = () => r.getByRole("button", { name: "Add to Fruit" });
  const box = () => r.getByRole("combobox", { name: "Add to Fruit" }) as HTMLInputElement;
  const type = (text: string) => {
    box().value = text;
    fireEvent.input(box());
  };
  const chips = () => [...r.container.querySelectorAll(".chip")].map((c) => c.getAttribute("data-value"));
  return { ...r, values, onReject, plus, box, type, chips };
}

describe("ChipPicker", () => {
  it("adds a picked suggestion and keeps the box open for the next", async () => {
    const m = mount([]);
    expect(m.queryByRole("combobox")).toBeNull();
    fireEvent.click(m.plus());
    expect(m.box()).toHaveFocus();
    m.type("ban");
    fireEvent.click(await m.findByRole("option", { name: "Banana" }));
    expect(m.values()).toEqual(["banana"]);
    expect(m.box()).toBeInTheDocument();
    m.type("");
    // A picked value is not suggested again.
    await waitFor(() => expect(m.getAllByRole("option").map((o) => o.textContent)).toEqual(["Apple", "Cherry"]));
    fireEvent.keyDown(m.box(), { key: "ArrowDown" });
    fireEvent.keyDown(m.box(), { key: "Enter" });
    expect(m.values()).toEqual(["banana", "apple"]);
  });

  it("removes a chip with its ×, and the last chip with Backspace in the empty box", () => {
    const m = mount(["apple", "banana", "cherry"], { chipText: (v) => v.toUpperCase() });
    fireEvent.click(m.getByRole("button", { name: "Remove BANANA" }));
    expect(m.values()).toEqual(["apple", "cherry"]);
    fireEvent.click(m.plus());
    fireEvent.keyDown(m.box(), { key: "Backspace" });
    expect(m.values()).toEqual(["apple"]);
    expect(m.chips()).toEqual(["apple"]);
  });

  it("Escape with no list showing puts the + back and focuses it", async () => {
    const m = mount([]);
    fireEvent.click(m.plus());
    m.type("zzz");
    fireEvent.keyDown(m.box(), { key: "Escape" });
    await waitFor(() => expect(m.queryByRole("combobox")).toBeNull());
    await waitFor(() => expect(m.plus()).toHaveFocus());
  });

  it("Escape with a list showing closes only the list", async () => {
    const m = mount([]);
    fireEvent.click(m.plus());
    m.type("a");
    await m.findByRole("listbox");
    fireEvent.keyDown(m.box(), { key: "Escape" });
    expect(m.queryByRole("listbox")).toBeNull();
    expect(m.box()).toBeInTheDocument();
  });

  it("with free text, typed text becomes a chip", () => {
    const m = mount([], { freeText: true, suggest: () => [] });
    fireEvent.click(m.plus());
    m.type(" kiwi ");
    fireEvent.keyDown(m.box(), { key: "Enter" });
    m.type("kiwi");
    fireEvent.keyDown(m.box(), { key: "Enter" });
    expect(m.values()).toEqual(["kiwi"]);
  });

  it("without free text, takes only a suggestion's text, matched without case, and rejects the rest", async () => {
    const m = mount([]);
    fireEvent.click(m.plus());
    m.type("kiwi");
    fireEvent.keyDown(m.box(), { key: "Enter" });
    await waitFor(() => expect(m.onReject).toHaveBeenCalledWith("kiwi"));
    expect(m.values()).toEqual([]);
    m.type("CHERRY");
    fireEvent.keyDown(m.box(), { key: "Enter" });
    await waitFor(() => expect(m.values()).toEqual(["cherry"]));
  });

  it("single: one chip, the + hidden while it is set, and its × clears it", async () => {
    const m = mount([], { single: true });
    fireEvent.click(m.plus());
    m.type("app");
    fireEvent.click(await m.findByRole("option", { name: "Apple" }));
    expect(m.values()).toEqual(["apple"]);
    expect(m.queryByRole("combobox")).toBeNull();
    expect(m.queryByRole("button", { name: "Add to Fruit" })).toBeNull();
    fireEvent.click(m.getByRole("button", { name: "Remove apple" }));
    expect(m.values()).toEqual([]);
    expect(m.plus()).toBeInTheDocument();
  });

  it("disabled: no +, no ×, and a dash when empty", () => {
    const m = mount(["apple"], { disabled: true });
    expect(m.queryByRole("button")).toBeNull();
    const e = mount([], { disabled: true });
    expect(e.getAllByText("—").length).toBeGreaterThan(0);
  });

  it("chips link when given an href", () => {
    const m = mount(["apple"], { chipHref: (v) => `/notes/${v}` });
    expect(m.getByRole("link", { name: "apple" })).toHaveAttribute("href", "/notes/apple");
  });

  it("shows hierarchy depth in suggestions", async () => {
    const leaf: Term = { name: "Wiring", path: "Plant/Electrical/Wiring", colour: null, description: null, children: [] };
    const mid: Term = { name: "Electrical", path: "Plant/Electrical", colour: null, description: null, children: [leaf] };
    const terms: Term[] = [{ name: "Plant", path: "Plant", colour: null, description: null, children: [mid] }];
    expect(termSuggestions(terms, "").map((s) => [s.value, s.depth])).toEqual([
      ["Plant", 0],
      ["Plant/Electrical", 1],
      ["Plant/Electrical/Wiring", 2],
    ]);
    // Matched by name or path; a nested match keeps its depth and says what it is under.
    expect(termSuggestions(terms, "wir")).toEqual([
      { value: "Plant/Electrical/Wiring", text: "Wiring", depth: 2, hint: "Plant/Electrical" },
    ]);
    expect(termSuggestions(terms, "electrical/").map((s) => s.value)).toEqual(["Plant/Electrical/Wiring"]);

    const m = mount([], { suggest: (q) => termSuggestions(terms, q) });
    fireEvent.click(m.plus());
    m.type("");
    const wiring = await m.findByRole("option", { name: "Wiring" });
    expect(wiring.style.paddingLeft).toBe(`${0.6 + 2 * 0.9}rem`);
    fireEvent.click(wiring);
    expect(m.values()).toEqual(["Plant/Electrical/Wiring"]);

    // A picked term with terms under it still shows, for them, but cannot be picked again.
    const held = mount(["Plant/Electrical"], { suggest: (q) => termSuggestions(terms, q) });
    fireEvent.click(held.plus());
    held.type("");
    const electrical = await held.findByRole("option", { name: "Electrical" });
    expect(electrical).toHaveAttribute("aria-disabled", "true");
    fireEvent.click(electrical);
    expect(held.values()).toEqual(["Plant/Electrical"]);
    fireEvent.keyDown(held.box(), { key: "ArrowDown" });
    fireEvent.keyDown(held.box(), { key: "ArrowDown" });
    fireEvent.keyDown(held.box(), { key: "Enter" });
    expect(held.values()).toEqual(["Plant/Electrical", "Plant/Electrical/Wiring"]);
  });

  it("matches types by name, label or plural, without case", () => {
    const t = (name: string, label: string, plural: string) => ({
      name,
      label,
      plural,
      description: null,
      icon: "check",
      colour: null,
      fields: [],
      templates: [],
    });
    const types = [t("task", "Task", "Tasks"), t("person", "Person", "People")];
    expect(typeSuggestions(types, "PEO").map((s) => s.value)).toEqual(["person"]);
    expect(typeSuggestions(types, "tas")).toEqual([{ value: "task", text: "Task", icon: "check" }]);
    expect(typeSuggestions(types, "").length).toBe(2);
  });
});
