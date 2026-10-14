import { describe, expect, it } from "vitest";
import type { Block, TypeDef } from "../api";
import {
  addView,
  newQueryView,
  readQueryView,
  removeView,
  renameView,
  setDefaultView,
  setViewItem,
  setViewKind,
  viewTemplate,
} from "./model";

const block: Block = {
  kind: "query_view",
  fields: { title: "Open", query: "from task where project == this", default_view: "board" },
  blocks: [
    { kind: "view", labels: ["open"], fields: { kind: "table", columns: ["title", "due"] } },
    { kind: "view", labels: ["board"], fields: { kind: "cards", title: "Board", template: "card" } },
    {
      kind: "view",
      labels: ["canvas"],
      fields: { kind: "canvas" },
      blocks: [{ kind: "place", labels: ["01JB8Z4K2MQ7V3X9T5R6W8Y2ZC"], fields: { x: 0, y: 0 } }],
    },
  ],
};

describe("readQueryView", () => {
  it("reads the query, the views and the view shown first", () => {
    const def = readQueryView(block);
    expect(def.title).toBe("Open");
    expect(def.query).toBe("from task where project == this");
    expect(def.views.map((v) => [v.name, v.kind, v.title])).toEqual([
      ["open", "table", "open"],
      ["board", "cards", "Board"],
      ["canvas", "canvas", "canvas"],
    ]);
    expect(def.views[0]?.columns).toEqual(["title", "due"]);
    expect(def.views[1]?.columns).toEqual(["title"]);
    expect(def.first?.name).toBe("board");
  });

  it("reads a kanban's group and a calendar's date and mode", () => {
    const views = readQueryView({
      kind: "query_view",
      fields: { query: "from task" },
      blocks: [
        { kind: "view", labels: ["board"], fields: { kind: "kanban", group: "stage" } },
        { kind: "view", labels: ["cal"], fields: { kind: "calendar", date: "due", mode: "week" } },
        { kind: "view", labels: ["line"], fields: { kind: "timeline", date: "span" } },
      ],
    }).views;
    expect(views.map((v) => [v.group, v.date, v.mode])).toEqual([
      ["stage", null, "month"],
      [null, "due", "week"],
      [null, "span", "month"],
    ]);
  });

  it("shows the first view when default_view names none", () => {
    expect(readQueryView({ ...block, fields: { query: "from *" } }).first?.name).toBe("open");
  });
});

describe("editing views", () => {
  it("renaming the default view keeps it the default", () => {
    const next = renameView(block, 1, "kanban");
    expect(next.blocks?.[1]?.labels).toEqual(["kanban"]);
    expect(next.fields?.default_view).toBe("kanban");
    expect(renameView(block, 0, "rows").fields?.default_view).toBe("board");
  });

  it("changing the kind drops items the new kind does not use", () => {
    const toList = setViewKind(block, 0, "list");
    expect(toList.blocks?.[0]).toEqual({ kind: "view", labels: ["open"], fields: { kind: "list" } });
    const toCards = setViewKind(block, 1, "details");
    expect(toCards.blocks?.[1]?.fields).toEqual({ kind: "details", title: "Board", template: "card" });
    expect(setViewKind(block, 2, "table").blocks?.[2]?.blocks).toBeUndefined();
    expect(setViewKind(block, 2, "canvas").blocks?.[2]?.blocks).toHaveLength(1);
  });

  it("adds a view with a fresh name and removes one, clearing a default that named it", () => {
    const added = addView(newQueryView());
    expect(added.blocks?.map((b) => b.labels?.[0])).toEqual(["table", "view2"]);
    const removed = removeView(block, 1);
    expect(removed.blocks?.map((b) => b.labels?.[0])).toEqual(["open", "canvas"]);
    expect(removed.fields?.default_view).toBeUndefined();
  });

  it("sets an item, and naming the first view as default leaves default_view out", () => {
    expect(setViewItem(block, 0, "columns", undefined).blocks?.[0]?.fields).toEqual({ kind: "table" });
    expect(setDefaultView(block, "open").fields?.default_view).toBeUndefined();
    expect(setDefaultView(block, "canvas").fields?.default_view).toBe("canvas");
  });
});

describe("viewTemplate", () => {
  const tab = (title: string) => ({ title, items: [] });
  const type = (names: string[]): TypeDef => ({
    name: "task",
    label: "Task",
    plural: "Tasks",
    description: null,
    icon: null,
    colour: null,
    fields: [],
    templates: names.map((name) => ({ name, label: null, tabs: [tab(`${name} 1`), tab(`${name} 2`)] })),
  });

  it("uses the named template, then card for cards or default for details", () => {
    expect(viewTemplate(type(["default", "card"]), "card", "default")?.name).toBe("card");
    expect(viewTemplate(type(["default", "card"]), null, "card")?.name).toBe("card");
    expect(viewTemplate(type(["default", "card"]), "missing", "default")?.name).toBe("default");
    expect(viewTemplate(type(["default"]), null, "default")?.tabs).toHaveLength(2);
  });

  it("gives a card only the first tab", () => {
    expect(viewTemplate(type(["default"]), null, "card")?.tabs.map((t) => t.title)).toEqual(["default 1"]);
    expect(viewTemplate(type(["default", "card"]), null, "card")?.tabs).toHaveLength(1);
  });
});
