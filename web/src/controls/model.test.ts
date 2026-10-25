import { describe, expect, it } from "vitest";
import type { FieldDef, NoteDoc, TypeDef } from "../api";
import { unplacedFields } from "../notes";
import {
  CONTROL_KINDS,
  type ControlKind,
  addDays,
  calendarPick,
  chipsAdd,
  chipsRemove,
  controlFits,
  controlOf,
  controlProblems,
  controlledFields,
  dayState,
  fittingFields,
  monthGrid,
  newControl,
  parseNumber,
  ratingKey,
  ratingMax,
  ratingValue,
  refusal,
  sameDayMonths,
  stepNumber,
  timeOf,
  withControlItem,
  withTime,
} from "./model";

const field = (name: string, kind: string, extra: Partial<FieldDef> = {}): FieldDef => ({
  name,
  label: name[0]!.toUpperCase() + name.slice(1),
  description: null,
  kind,
  required: false,
  computed: false,
  multiple: false,
  compound: ["content", "dict", "ink", "canvas"].includes(kind),
  ...extra,
});

const FIELDS: FieldDef[] = [
  field("name", "string"),
  field("site", "url"),
  field("count", "int"),
  field("ratio", "float"),
  field("done", "bool"),
  field("due", "date"),
  field("at", "datetime"),
  field("span", "date_range", { of: "date" }),
  field("slots", "date_range", { of: "datetime" }),
  // A range of either form fits no control: the calendar control picks days only.
  field("when", "date_range", { of: "date_or_datetime" }),
  field("size", "select", { options: ["s", "m"] }),
  field("sizes", "select", { options: ["s", "m"], multiple: true }),
  field("area", "taxonomy", { taxonomy: "area" }),
  field("areas", "taxonomy", { taxonomy: "area", multiple: true }),
  field("parent", "reference", { target: "t" }),
  field("links", "reference", { target: "t", multiple: true }),
  field("tags", "list", { items: "string" }),
  field("numbers", "list", { items: "int" }),
  // A list of labelled values: chips with their labels.
  field("phones", "list", {
    items: "dict",
    item: field("phones", "dict", { compound: false, keys: [field("number", "string"), field("label", "string")], values: null }),
  }),
  // Any other list of dicts fits no control.
  field("contacts", "list", {
    items: "dict",
    item: field("contacts", "dict", { compound: false, keys: [field("number", "string"), field("kind", "string")], values: null }),
  }),
  field("extra", "dict", { values: { ...field("extra", "string"), kind: "string" } }),
  field("place", "location"),
  field("body", "content"),
  field("cover", "asset"),
  field("sketch", "ink"),
  field("plan", "canvas"),
];

/** The fit table, as docs/vault-format.md gives it. */
const FITS: Record<ControlKind, string[]> = {
  text: ["name", "site", "count", "ratio"],
  textarea: ["name"],
  number: ["count", "ratio"],
  slider: ["count", "ratio"],
  toggle: ["done"],
  checkbox: ["done"],
  dropdown: ["size", "area", "parent"],
  radio: ["size"],
  chips: ["sizes", "areas", "links", "tags", "phones"],
  calendar: ["due", "span"],
  datetime: ["at"],
  time: ["at"],
  rating: ["count"],
};

describe("the fit table", () => {
  it("covers every kind", () => {
    expect(Object.keys(FITS)).toEqual([...CONTROL_KINDS]);
  });

  for (const kind of CONTROL_KINDS) {
    it(`${kind} fits only its fields`, () => {
      expect(fittingFields(kind, FIELDS).map((f) => f.name)).toEqual(FITS[kind]);
      for (const f of FIELDS) expect(controlFits(kind, f)).toBe(FITS[kind].includes(f.name));
    });
  }
});

describe("control items", () => {
  it("reads a control block leniently", () => {
    expect(controlOf({ kind: "control", fields: { field: "count", kind: "slider", min: 0, max: 5, label: "Mix" } })).toEqual({
      field: "count",
      kind: "slider",
      label: "Mix",
      placeholder: null,
      min: 0,
      max: 5,
      step: undefined,
    });
    expect(controlOf({ kind: "control", fields: { field: "count", kind: "spinner" } })).toBeNull();
    expect(controlOf({ kind: "paragraph", fields: { text: "x" } })).toBeNull();
  });

  it("writes items in the documented order and drops cleared ones", () => {
    let b = newControl("slider", "ratio");
    expect(b.fields).toEqual({ field: "ratio", kind: "slider", min: 0, max: 100, step: 1 });
    b = withControlItem(b, "label", "Mix");
    expect(Object.keys(b.fields!)).toEqual(["field", "kind", "label", "min", "max", "step"]);
    b = withControlItem(b, "step", undefined);
    expect(b.fields).toEqual({ field: "ratio", kind: "slider", label: "Mix", min: 0, max: 100 });
    expect(newControl("rating", "count").fields).toEqual({ field: "count", kind: "rating" });
  });

  it("says what is wrong with a control's own items", () => {
    const count = FIELDS.find((f) => f.name === "count")!;
    const spec = (fields: Record<string, number | string>) => controlOf({ kind: "control", fields: { field: "count", ...fields } })!;
    expect(controlProblems(spec({ kind: "slider", min: 0, max: 5 }), count)).toEqual([]);
    expect(controlProblems(spec({ kind: "slider", min: 0 }), count)).toEqual(["A slider needs a minimum and a maximum."]);
    expect(controlProblems(spec({ kind: "slider", min: 5, max: 5 }), count)).toEqual(["The maximum must be greater than the minimum."]);
    expect(controlProblems(spec({ kind: "number", step: 0.5 }), count)).toEqual(["The step must be a whole number for an int field."]);
    expect(controlProblems(spec({ kind: "rating", max: 11 }), count)).toEqual(["A rating has 1 to 10 stars."]);
    expect(controlProblems(spec({ kind: "toggle" }), count)[0]).toMatch(/does not fit Count, a int field/);
    expect(controlProblems(spec({ kind: "text" }), undefined)).toEqual(["The type has no field count."]);
  });
});

describe("value mapping", () => {
  it("text and numbers", () => {
    expect(parseNumber("", true)).toBeUndefined();
    expect(parseNumber(" 12 ", true)).toBe(12);
    expect(parseNumber("1.5", true)).toBeNull();
    expect(parseNumber("1.5", false)).toBe(1.5);
    expect(parseNumber("abc", false)).toBeNull();
  });

  it("a stepper steps on its grid within its ends", () => {
    const s = { min: 0, max: 10, step: 2 };
    expect(stepNumber(undefined, 1, s, true)).toBe(0);
    expect(stepNumber(0, 1, s, true)).toBe(2);
    expect(stepNumber(3, 1, s, true)).toBe(4);
    expect(stepNumber(3, -1, s, true)).toBe(2);
    expect(stepNumber(10, 1, s, true)).toBe(10);
    expect(stepNumber(0, -1, s, true)).toBe(0);
    expect(stepNumber(0.2, 1, { step: 0.1 }, false)).toBe(0.3);
    expect(stepNumber(undefined, -1, { max: 5 }, true)).toBe(5);
  });

  it("time of a datetime keeps its date and offset, or takes today", () => {
    expect(timeOf("2026-10-20T09:30:15+11:00")).toBe("09:30");
    expect(timeOf(undefined)).toBe("");
    expect(withTime("2026-10-20T09:30:15+11:00", "14:45")).toBe("2026-10-20T14:45:00+11:00");
    expect(withTime("2026-10-20T09:30:15Z", "07:05:09")).toBe("2026-10-20T07:05:09Z");
    const now = new Date(2026, 0, 2, 12, 0, 0);
    expect(withTime(undefined, "08:00", now)).toMatch(/^2026-01-02T08:00:00(Z|[+-]\d\d:\d\d)$/);
    expect(withTime("2026-10-20T09:30:15Z", "")).toBeUndefined();
  });

  it("a date takes one click and a range two, in either order", () => {
    expect(calendarPick(false, null, "2026-10-05")).toEqual({ write: true, value: "2026-10-05", pending: null });
    const first = calendarPick(true, null, "2026-10-20");
    expect(first).toEqual({ write: false, pending: "2026-10-20" });
    expect(calendarPick(true, "2026-10-20", "2026-10-10")).toEqual({
      write: true,
      value: { start: "2026-10-10", end: "2026-10-20" },
      pending: null,
    });
    expect(calendarPick(true, "2026-10-10", "2026-10-10")).toMatchObject({ value: { start: "2026-10-10", end: "2026-10-10" } });
  });

  it("marks the chosen day, a range's ends and the days inside it", () => {
    const range = { start: "2026-10-10", end: "2026-10-12" };
    expect(dayState(range, null, "2026-10-10")).toBe("start");
    expect(dayState(range, null, "2026-10-11")).toBe("inside");
    expect(dayState(range, null, "2026-10-12")).toBe("end");
    expect(dayState(range, null, "2026-10-13")).toBeNull();
    expect(dayState(range, "2026-10-20", "2026-10-11")).toBeNull();
    expect(dayState(range, "2026-10-20", "2026-10-20")).toBe("start");
    expect(dayState("2026-10-05", null, "2026-10-05")).toBe("selected");
  });

  it("a month grid runs Monday to Sunday and covers the month", () => {
    const weeks = monthGrid("2026-10-14");
    expect(weeks.every((w) => w.length === 7)).toBe(true);
    // October 2026 starts on a Thursday.
    expect(weeks[0]).toEqual(["2026-09-28", "2026-09-29", "2026-09-30", "2026-10-01", "2026-10-02", "2026-10-03", "2026-10-04"]);
    expect(weeks.at(-1)).toContain("2026-10-31");
    expect(addDays("2026-12-31", 1)).toBe("2027-01-01");
    expect(sameDayMonths("2026-01-31", 1)).toBe("2026-02-28");
  });

  it("chips add once and remove to no value", () => {
    expect(chipsAdd(undefined, "a")).toEqual(["a"]);
    expect(chipsAdd(["a"], " b ")).toEqual(["a", "b"]);
    expect(chipsAdd(["a"], "a")).toEqual(["a"]);
    expect(chipsAdd(["a"], "  ")).toEqual(["a"]);
    expect(chipsRemove(["a", "b"], "a")).toEqual(["b"]);
    expect(chipsRemove(["a"], "a")).toBeUndefined();
  });

  it("a rating is set by a star, cleared by the same star, and moved by keys", () => {
    expect(ratingValue(undefined, 3)).toBe(3);
    expect(ratingValue(3, 4)).toBe(4);
    expect(ratingValue(3, 3)).toBeUndefined();
    expect(ratingKey(undefined, "ArrowRight", 5)).toBe(1);
    expect(ratingKey(5, "ArrowRight", 5)).toBe(5);
    expect(ratingKey(2, "ArrowLeft", 5)).toBe(1);
    expect(ratingKey(1, "ArrowLeft", 5)).toBe(1);
    expect(ratingKey(2, "End", 7)).toBe(7);
    expect(ratingKey(2, "Home", 7)).toBe(1);
    expect(ratingKey(2, "a", 7)).toBeNull();
    expect(ratingMax({})).toBe(5);
    expect(ratingMax({ max: 10 })).toBe(10);
    expect(ratingMax({ max: 40 })).toBe(5);
  });

  it("a required field cannot be emptied", () => {
    const req = field("name", "string", { required: true });
    expect(refusal(req, "Name", undefined)).toBe("Name is required, so it cannot be empty.");
    expect(refusal(req, "Name", [])).not.toBeNull();
    expect(refusal(req, "Name", "x")).toBeNull();
    expect(refusal(field("notes", "string"), "Notes", undefined)).toBeNull();
    expect(refusal(field("done", "bool", { required: true }), "Done", false)).toBeNull();
  });
});

describe("unplaced fields with controls", () => {
  const words = field("words", "int", { computed: true });
  const type = (blocks: object[]): TypeDef => ({
    name: "t",
    label: "T",
    plural: "Ts",
    description: null,
    icon: null,
    colour: null,
    fields: [...FIELDS.filter((f) => ["name", "count", "done", "due", "size", "body", "plan"].includes(f.name)), words],
    templates: [{ name: "default", label: null, tabs: [{ title: "Main", items: [{ slot: "body", label: null }, ...blocks.map((b) => ({ block: b as never }))] }] }],
  });
  const ctl = (f: string, kind: string) => ({ kind: "control", fields: { field: f, kind } });
  const names = (fs: FieldDef[]) => fs.map((f) => f.name);

  it("a control in the template places its field, in columns too", () => {
    const t = type([
      ctl("name", "text"),
      { kind: "columns", blocks: [{ kind: "column", blocks: [ctl("count", "slider")] }, { kind: "column", blocks: [ctl("done", "toggle")] }] },
    ]);
    expect(names(unplacedFields(t))).toEqual(["due", "size", "plan", "words"]);
    expect([...controlledFields(t)].sort()).toEqual(["count", "done", "name"]);
  });

  it("a read-only control on a computed field, or one that does not fit, places nothing", () => {
    const t = type([ctl("words", "number"), ctl("due", "toggle"), ctl("gone", "text")]);
    expect(names(unplacedFields(t))).toEqual(["name", "count", "done", "due", "size", "plan", "words"]);
  });

  it("a control in the note's own content or a canvas card places its field", () => {
    const t = type([]);
    const doc: NoteDoc = {
      fields: { title: "x" },
      blocks: [
        { kind: "content", labels: ["body"], blocks: [ctl("size", "radio")] },
        {
          kind: "canvas",
          labels: ["plan"],
          blocks: [{ kind: "node", labels: ["a"], fields: { kind: "card" }, blocks: [ctl("due", "calendar")] }],
        },
      ],
    };
    expect(names(unplacedFields(t, doc))).toEqual(["name", "count", "done", "plan", "words"]);
    expect(names(unplacedFields(t))).toEqual(["name", "count", "done", "due", "size", "plan", "words"]);
  });
});
