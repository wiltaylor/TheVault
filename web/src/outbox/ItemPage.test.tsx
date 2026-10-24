import { MemoryRouter, Route } from "@solidjs/router";
import { cleanup, fireEvent, render, waitFor } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { FieldDef, Note, TemplateItem, TypeDef } from "../api";

// An Outbox item's own page (outbox X13, X14): one form for its kind, with the body under the email
// or SMS fields, the status shown as a chip and never chosen, an AI chip beside it for an agent's
// draft, and the Outbox buttons: Ready on a draft, Retry on a failed item, Delete on both.

const note = vi.fn();
const updateNote = vi.fn();
vi.mock("../api", async (orig) => {
  const real = await orig<typeof import("../api")>();
  return { ...real, api: { ...real.api, note, updateNote, backlinks: () => Promise.resolve([]) } };
});

const types: TypeDef[] = [];
vi.mock("../session", async (orig) => {
  const real = await orig<typeof import("../session")>();
  return { ...real, typeDef: (name: string) => types.find((t) => t.name === name) };
});

const { NoteView } = await import("../pages/NoteView");

const field = (name: string, label: string, kind: string, more: Partial<FieldDef> = {}): FieldDef => ({
  name,
  label,
  description: null,
  kind,
  required: false,
  computed: false,
  multiple: false,
  compound: kind === "content",
  ...more,
});

const EMAIL = 'kind == "email"';
const SMS = 'kind == "sms"';
const slot = (name: string, when?: string): TemplateItem => ({ slot: name, label: null, ...(when ? { when } : {}) });

/** The shipped `outbox` type's fields and default template (crates/writer/defaults/outbox.wcl), cut down. */
const OUTBOX_TYPE: TypeDef = {
  name: "outbox",
  label: "Outbox item",
  plural: "Outbox",
  description: null,
  icon: null,
  colour: null,
  fields: [
    field("kind", "Kind", "select", { options: ["email", "sms", "event"] }),
    field("status", "Status", "select", { options: ["draft", "ready", "sending", "failed"] }),
    field("to", "To", "list", { items: "string" }),
    field("subject", "Subject", "string"),
    field("phone", "Phone number", "string"),
    field("body", "Body", "content"),
    field("error", "Error", "string"),
  ],
  templates: [
    {
      name: "default",
      label: null,
      tabs: [
        {
          title: "Outbox item",
          items: [slot("status"), slot("to", EMAIL), slot("subject", EMAIL), slot("phone", SMS), slot("body", 'kind != "event"'), slot("error", "!empty(error)")],
        },
      ],
    },
  ],
};

const ID = "01JB8Z4K2MQ7V3X9T5R6W8Y2T1";

function mount(fields: Record<string, unknown>) {
  types.splice(0, types.length, OUTBOX_TYPE);
  const stored: Note = {
    id: ID,
    type: "outbox",
    version: "v1",
    wcl: "",
    note: {
      fields: { title: "Lunch", drafted_by: "user", ...fields } as Note["note"]["fields"],
      blocks: [{ kind: "content", labels: ["body"], blocks: [{ kind: "paragraph", fields: { text: "Noon?" } }] }],
    },
  };
  note.mockResolvedValue(stored);
  updateNote.mockReturnValue(new Promise(() => {}));
  return render(() => (
    <MemoryRouter>
      <Route path="*" component={() => <NoteView id={ID} />} />
    </MemoryRouter>
  ));
}

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe("an Outbox item's page", () => {
  it("shows an email's fields and its body in one form, and its status with no dropdown", async () => {
    const m = mount({ kind: "email", status: "draft", to: ["ada@example.com"] });
    await waitFor(() => expect(m.getByTestId("field-status").textContent).toContain("Draft"));
    expect(m.queryByRole("combobox", { name: "Status" })).toBeNull();
    expect(m.getByTestId("field-status").querySelector("select")).toBeNull();
    expect(m.queryAllByRole("tab")).toHaveLength(0);
    expect(m.getByTestId("field-to")).toBeTruthy();
    expect(m.getByTestId("field-subject")).toBeTruthy();
    expect(m.getByTestId("field-body")).toBeTruthy();
    expect(m.queryByTestId("field-phone")).toBeNull();
    expect(m.queryByTestId("field-error")).toBeNull();
  });

  it("shows an SMS's number and its body, and none of the email fields", async () => {
    const m = mount({ kind: "sms", status: "draft", phone: "+61400000001" });
    await waitFor(() => expect(m.getByTestId("field-phone")).toBeTruthy());
    expect(m.getByTestId("field-body")).toBeTruthy();
    expect(m.queryByTestId("field-to")).toBeNull();
    expect(m.queryByTestId("field-subject")).toBeNull();
  });

  it("Ready on a draft marks it ready through the page's save", async () => {
    const m = mount({ kind: "email", status: "draft", to: ["ada@example.com"] });
    fireEvent.click(await m.findByRole("button", { name: "Ready" }));
    await waitFor(() => expect(updateNote).toHaveBeenCalled());
    const [id, doc] = updateNote.mock.calls.at(-1)!;
    expect(id).toBe(ID);
    expect(doc.fields.status).toBe("ready");
  });

  it("offers Ready and Delete on a draft, Retry and Delete with the error once failed, and nothing to approve", async () => {
    const m = mount({ kind: "email", status: "draft" });
    await m.findByRole("button", { name: "Ready" });
    expect(m.getByRole("button", { name: "Delete this item" })).toBeTruthy();
    expect(m.queryByRole("button", { name: "Approve and send" })).toBeNull();
    expect(m.queryByRole("button", { name: "Reject" })).toBeNull();
    cleanup();
    const f = mount({ kind: "sms", status: "failed", error: "Turn on SMS sending" });
    await f.findByRole("button", { name: "Retry" });
    expect(f.getByRole("button", { name: "Delete this item" })).toBeTruthy();
    expect(f.getByTestId("field-error")).toBeTruthy();
    expect(f.getByTestId("field-status").textContent).toContain("Error");
    cleanup();
    const r = mount({ kind: "email", status: "ready" });
    await waitFor(() => expect(r.getByTestId("field-status").textContent).toContain("Ready"));
    expect(r.queryByRole("button", { name: "Ready" })).toBeNull();
    expect(r.queryByRole("button", { name: "Delete this item" })).toBeNull();
  });

  it("shows the status as a chip with an AI chip beside it for an agent's draft, and never who drafted it", async () => {
    const m = mount({ kind: "email", status: "draft", drafted_by: "clerk" });
    await waitFor(() => expect(m.getByTestId("field-status").querySelector(".outbox-chip.outbox-status")?.textContent).toBe("Draft"));
    const ai = m.getByTestId("field-status").querySelector(".outbox-ai-chip");
    expect(ai?.textContent).toBe("AI");
    expect(ai?.getAttribute("title")).toBe("Drafted by clerk");
    expect(m.container.textContent).not.toContain("Drafted by");
    cleanup();
    const u = mount({ kind: "email", status: "draft", drafted_by: "user" });
    await waitFor(() => expect(u.getByTestId("field-status").textContent).toContain("Draft"));
    expect(u.getByTestId("field-status").querySelector(".outbox-ai-chip")).toBeNull();
  });
});
