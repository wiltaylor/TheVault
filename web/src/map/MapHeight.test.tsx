import { MemoryRouter, Route } from "@solidjs/router";
import { cleanup, fireEvent, render, waitFor } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Block, FieldDef, Page, TypeDef } from "../api";

// How tall a map is: a map view's `height` item, set by the handle on its bottom edge and saved
// with the block, and a location field's map, whose height this browser keeps per type and field.

const updateSize = vi.fn();
const made = vi.fn();
vi.mock("./ol", () => ({
  createMap: () => (made(), {
    setPins: () => {},
    showPopup: () => {},
    popup: document.createElement("div"),
    setTiles: () => {},
    updateSize,
    dispose: () => {},
  }),
}));

const query = vi.fn();
vi.mock("../api", async (orig) => {
  const real = await orig<typeof import("../api")>();
  return { ...real, api: { ...real.api, query } };
});

const placeField: FieldDef = {
  name: "place",
  label: "Place",
  description: null,
  kind: "location",
  required: false,
  computed: false,
  multiple: false,
  compound: false,
};

const types: TypeDef[] = [
  {
    name: "site",
    label: "Site",
    plural: "Sites",
    description: null,
    icon: null,
    colour: null,
    fields: [placeField],
    templates: [],
  },
];
vi.mock("../session", async (orig) => {
  const real = await orig<typeof import("../session")>();
  return { ...real, typeDef: (name: string) => types.find((t) => t.name === name) };
});

const { QueryView } = await import("../views/QueryView");
const { FieldInput } = await import("../components/FieldInput");
const { LOCATION_MAP_HEIGHT, MAP_VIEW_HEIGHT } = await import("./model");

function mapBlock(height?: number): Block {
  return {
    kind: "query_view",
    fields: { query: "from site" },
    blocks: [
      { kind: "view", labels: ["list"], fields: { kind: "list" } },
      { kind: "view", labels: ["where"], fields: { kind: "map", location: "place", ...(height ? { height } : {}) } },
    ],
  };
}

const pointer = (clientY: number) => ({ pointerId: 3, pointerType: "mouse", button: 0, clientY });

/** The query view, on its map tab, keeping each block it saves as the one it shows. */
function mountView(initial: Block, editable = true) {
  const [block, setBlock] = createSignal(initial);
  const saved: Block[] = [];
  const view = () => (
    <QueryView
      block={block()}
      onChange={
        editable
          ? (b) => {
              saved.push(b);
              setBlock(b);
            }
          : undefined
      }
    />
  );
  const r = render(() => (
    <MemoryRouter>
      <Route path="*" component={view} />
    </MemoryRouter>
  ));
  return { r, saved };
}

async function openMap(r: ReturnType<typeof render>) {
  fireEvent.click(await r.findByRole("tab", { name: "where" }));
  return r.findByTestId("map");
}

const mapViewOf = (b: Block) => b.blocks!.find((v) => v.labels?.[0] === "where")!;

beforeEach(() => {
  updateSize.mockClear();
  made.mockClear();
  localStorage.clear();
  query.mockImplementation(
    async (): Promise<Page> => ({
      rows: [{ id: "s1", type: "site", title: "Shed", fields: { place: { lat: -37.8, lng: 144.9 } } }],
      total: 1,
      next: null,
    }),
  );
});

afterEach(cleanup);

describe("a map view's height", () => {
  it("is 448 pixels by default and resizes live while its handle is dragged", async () => {
    const { r, saved } = mountView(mapBlock());
    const map = await openMap(r);
    expect(map.style.height).toBe(`${MAP_VIEW_HEIGHT.initial}px`);
    await waitFor(() => expect(made).toHaveBeenCalled());
    const handle = r.getByRole("separator", { name: "Map height" });
    fireEvent.pointerDown(handle, pointer(100));
    fireEvent.pointerMove(handle, pointer(250));
    expect(map.style.height).toBe("598px");
    expect(updateSize).toHaveBeenCalled();
    expect(saved).toEqual([]);
    fireEvent.pointerMove(handle, pointer(400));
    fireEvent.pointerUp(handle, pointer(400));
    expect(saved).toHaveLength(1);
    expect(mapViewOf(saved[0]!).fields).toEqual({ kind: "map", location: "place", height: 748 });
    expect(map.style.height).toBe("748px");
  });

  it("goes back to the default on a double-click, which leaves the item out", async () => {
    const { r, saved } = mountView(mapBlock(900));
    const map = await openMap(r);
    expect(map.style.height).toBe("900px");
    fireEvent.dblClick(r.getByRole("separator", { name: "Map height" }));
    expect(saved).toHaveLength(1);
    expect(mapViewOf(saved[0]!).fields).toEqual({ kind: "map", location: "place" });
    expect(map.style.height).toBe("448px");
  });

  it("keeps to 160 to 4000 pixels", async () => {
    const { r, saved } = mountView(mapBlock());
    const map = await openMap(r);
    const handle = r.getByRole("separator", { name: "Map height" });
    fireEvent.pointerDown(handle, pointer(1000));
    fireEvent.pointerUp(handle, pointer(0));
    // The map is resized in place, not made again.
    expect(map.isConnected && handle.isConnected).toBe(true);
    fireEvent.pointerDown(handle, pointer(0));
    fireEvent.pointerUp(handle, pointer(10_000));
    expect(saved.map((b) => mapViewOf(b).fields?.height)).toEqual([160, 4000]);
    expect(made).toHaveBeenCalledTimes(1);
  });

  it("shows the stored height with no handle when the view is read-only", async () => {
    const { r } = mountView(mapBlock(700), false);
    const map = await openMap(r);
    expect(map.style.height).toBe("700px");
    expect(r.queryByRole("separator", { name: "Map height" })).toBeNull();
  });
});

describe("a location field's map height", () => {
  function mountField(type = "site") {
    const values: unknown[] = [];
    const r = render(() => (
      <FieldInput
        field={placeField}
        value={undefined}
        onChange={(v) => values.push(v)}
        label="Place"
        id="place"
        related={{ type, value: () => undefined, set: () => {} }}
      />
    ));
    return { r, values, map: () => r.getByTestId("map"), handle: () => r.getByRole("separator", { name: "Place map height" }) };
  }

  it("is 256 pixels by default, kept per type and field once dragged, and reset by a double-click", () => {
    const first = mountField();
    expect(first.map().style.height).toBe(`${LOCATION_MAP_HEIGHT.initial}px`);
    fireEvent.pointerDown(first.handle(), pointer(100));
    fireEvent.pointerMove(first.handle(), pointer(300));
    expect(first.map().style.height).toBe("456px");
    fireEvent.pointerUp(first.handle(), pointer(300));
    expect(first.map().style.height).toBe("456px");
    expect(first.values).toEqual([]);
    cleanup();

    // Another note of the type shows the same height; another type's field does not.
    expect(mountField().map().style.height).toBe("456px");
    cleanup();
    expect(mountField("depot").map().style.height).toBe("256px");
    cleanup();

    const again = mountField();
    fireEvent.dblClick(again.handle());
    expect(again.map().style.height).toBe("256px");
    cleanup();
    expect(mountField().map().style.height).toBe("256px");
    expect(Object.keys(localStorage)).toEqual([]);
  });

  it("keeps to 160 to 2000 pixels", () => {
    const { handle, map } = mountField();
    fireEvent.pointerDown(handle(), pointer(0));
    fireEvent.pointerUp(handle(), pointer(10_000));
    expect(map().style.height).toBe("2000px");
  });
});
