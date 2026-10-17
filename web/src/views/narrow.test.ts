import { describe, expect, it } from "vitest";
import type { FieldDef, Term, TypeDef } from "../api";
import {
  calendarEvent,
  dateField,
  dateWindow,
  fromTypes,
  groupField,
  kanbanColumns,
  narrow,
  quote,
  timelineItem,
  visibleWindow,
} from "./narrow";

describe("narrow", () => {
  it("adds a where clause before the first later clause", () => {
    expect(narrow("from task sort title asc", 'status == "Done"')).toBe(
      'from task where status == "Done" sort title asc',
    );
    expect(narrow("from task", "a")).toBe("from task where a");
    expect(narrow("from *  limit 5", "a")).toBe("from * where a limit 5");
  });

  it("joins an existing where clause in parentheses", () => {
    expect(narrow("from task where project == this sort title asc", "b")).toBe(
      "from task where (project == this) && (b) sort title asc",
    );
    expect(narrow("from task where a || c group by status", "b")).toBe(
      "from task where (a || c) && (b) group by status",
    );
  });

  it("skips clause words inside strings, member names and longer names", () => {
    expect(narrow('from task where title == "sort limit" sort title', "b")).toBe(
      'from task where (title == "sort limit") && (b) sort title',
    );
    expect(narrow('from task where note.sort == "a \\" where" && sort_order > 1', "b")).toBe(
      'from task where (note.sort == "a \\" where" && sort_order > 1) && (b)',
    );
    expect(narrow('from task where any(tags, |x| x == "y") search "where"', "b")).toBe(
      'from task where (any(tags, |x| x == "y")) && (b) search "where"',
    );
  });
});

describe("include archived", () => {
  it("keeps a new where clause after include archived", () => {
    expect(narrow("from task include archived", "a")).toBe("from task include archived where a");
    expect(narrow("from task include archived sort due", "a")).toBe("from task include archived where a sort due");
    expect(narrow("from task include archived where b limit 3", "a")).toBe(
      "from task include archived where (b) && (a) limit 3",
    );
  });

  it("is a clause only after the type list, so a field may be called include", () => {
    expect(narrow("from task where include sort title", "a")).toBe("from task where (include) && (a) sort title");
  });
});

describe("fromTypes", () => {
  it("reads the from clause, or null for every type", () => {
    expect(fromTypes("from task, project where a")).toEqual(["task", "project"]);
    expect(fromTypes("from * sort title")).toBeNull();
    expect(fromTypes("from task")).toEqual(["task"]);
    expect(fromTypes("from task, project include archived where a")).toEqual(["task", "project"]);
    expect(fromTypes("from * include archived")).toBeNull();
  });
});

const field = (name: string, kind: string, extra: Partial<FieldDef> = {}): FieldDef => ({
  name,
  label: name.charAt(0).toUpperCase() + name.slice(1),
  description: null,
  kind,
  required: false,
  computed: false,
  multiple: false,
  compound: false,
  ...extra,
});

const type = (name: string, fields: FieldDef[]): TypeDef => ({
  name,
  label: name,
  plural: name,
  description: null,
  icon: null,
  colour: null,
  fields,
  templates: [],
});

const types = [
  type("story", [
    field("stage", "taxonomy", { taxonomy: "stage" }),
    field("topics", "taxonomy", { taxonomy: "stage", multiple: true }),
    field("due", "date"),
    field("span", "date_range", { of: "date" }),
    field("at", "datetime"),
    field("estimate", "int"),
  ]),
  type("task", [field("status", "select")]),
];

describe("groupField", () => {
  it("finds a single-term taxonomy field on the query's types", () => {
    const r = groupField(types, "from story", "stage");
    expect(r.field?.name).toBe("stage");
    expect(r.problem).toBeNull();
  });

  it("refuses a multi-term field with a clear message", () => {
    const r = groupField(types, "from *", "topics");
    expect(r.field).toBeNull();
    expect(r.problem).toBe(
      "A kanban view needs a single-term taxonomy field, but Topics holds several terms. Choose a field that holds one term.",
    );
  });

  it("refuses a missing group, an unknown field and a field of another kind", () => {
    expect(groupField(types, "from story", null).problem).toBe(
      "A kanban view needs a group: a single-term taxonomy field whose terms are its columns.",
    );
    expect(groupField(types, "from task", "stage").problem).toBe("No type in this query has a field named stage.");
    expect(groupField(types, "from task", "status").problem).toBe(
      "A kanban view needs a single-term taxonomy field, but Status is a select field.",
    );
  });
});

describe("dateField", () => {
  it("accepts date, datetime and date range fields only", () => {
    expect(dateField(types, "from story", "due").field?.kind).toBe("date");
    expect(dateField(types, "from story", "span").field?.kind).toBe("date_range");
    expect(dateField(types, "from story", "estimate").problem).toBe(
      "This view needs a date, datetime or date range field, but Estimate is an int field.",
    );
    expect(dateField(types, "from story", null).problem).toBe(
      "This view needs a date: a date, datetime or date range field.",
    );
  });
});

const term = (name: string, path: string, children: Term[] = [], colour: string | null = null): Term => ({
  name,
  path,
  colour,
  description: null,
  children,
});

describe("kanbanColumns", () => {
  it("makes a column per term in display order, then one for notes with no term", () => {
    const terms = [
      term("Inbox", "Inbox", [], "gray"),
      term("Active", "Active", [term("Blocked", "Active/Blocked", [], "red")]),
      term("Done", "Done"),
    ];
    const cols = kanbanColumns(terms, "stage", "Stage");
    expect(cols.map((c) => [c.key, c.label, c.colour, c.value])).toEqual([
      ["Inbox", "Inbox", "gray", "Inbox"],
      ["Active", "Active", null, "Active"],
      ["Active/Blocked", "Active / Blocked", "red", "Active/Blocked"],
      ["Done", "Done", null, "Done"],
      ["", "No stage", null, undefined],
    ]);
    expect(cols[0]?.where).toBe('stage == "Inbox"');
    expect(cols[4]?.where).toBe('!(stage == "Inbox" || stage == "Active" || stage == "Active/Blocked" || stage == "Done")');
  });

  it("colours a term's column by its nearest coloured ancestor when it has no colour", () => {
    const terms = [term("Active", "Active", [term("Blocked", "Active/Blocked"), term("Late", "Active/Late", [], "red")], "blue")];
    expect(kanbanColumns(terms, "stage", "Stage").map((c) => c.colour)).toEqual(["blue", "blue", "red", null]);
  });

  it("quotes term names with quotes and backslashes", () => {
    expect(quote('a "b" \\ c')).toBe('"a \\"b\\" \\\\ c"');
  });
});

describe("dateWindow", () => {
  it("bounds a date field, a datetime field and a range field to a window", () => {
    expect(dateWindow(field("due", "date"), "2026-10-01", "2026-11-01")).toBe(
      'due >= "2026-10-01" && due < "2026-11-01"',
    );
    expect(dateWindow(field("span", "date_range"), "2026-10-01", "2026-11-01")).toBe(
      'span.start < "2026-11-01" && span.end >= "2026-10-01"',
    );
  });
});

describe("calendar and timeline items", () => {
  const row = (fields: Record<string, unknown>) => ({ id: "N1", type: "story", title: "One", fields }) as never;

  it("puts a date on its day and spans a date range over its days, end inclusive", () => {
    expect(calendarEvent(row({ due: "2026-10-05" }), field("due", "date"))).toMatchObject({
      id: "N1",
      title: "One",
      start: "2026-10-05",
      allDay: true,
    });
    expect(
      calendarEvent(row({ span: { start: "2026-10-05", end: "2026-10-07" } }), field("span", "date_range")),
    ).toMatchObject({ start: "2026-10-05", end: "2026-10-08", allDay: true });
    expect(
      calendarEvent(
        row({ span: { start: "2026-10-05T09:00:00Z", end: "2026-10-05T11:00:00Z" } }),
        field("span", "date_range", { of: "datetime" }),
      ),
    ).toMatchObject({ start: "2026-10-05T09:00:00Z", end: "2026-10-05T11:00:00Z", allDay: false });
    expect(calendarEvent(row({ at: "2026-10-05T09:00:00+11:00" }), field("at", "datetime"))).toMatchObject({
      start: "2026-10-05T09:00:00+11:00",
      allDay: false,
    });
    expect(calendarEvent(row({}), field("due", "date"))).toBeNull();
  });

  it("places a date_or_datetime range as all-day when it holds dates and as timed when it holds datetimes", () => {
    const when = field("when", "date_range", { of: "date_or_datetime" });
    expect(calendarEvent(row({ when: { start: "2026-10-05", end: "2026-10-06" } }), when)).toEqual({
      id: "N1",
      title: "One",
      start: "2026-10-05",
      end: "2026-10-07",
      allDay: true,
    });
    expect(
      calendarEvent(row({ when: { start: "2026-10-05T09:00:00+11:00", end: "2026-10-05T10:30:00+11:00" } }), when),
    ).toEqual({ id: "N1", title: "One", start: "2026-10-05T09:00:00+11:00", end: "2026-10-05T10:30:00+11:00", allDay: false });
    expect(timelineItem(row({ when: { start: "2026-10-05", end: "2026-10-06" } }), when)).toMatchObject({
      start: "2026-10-05",
      end: "2026-10-07",
      type: "range",
    });
    expect(timelineItem(row({ when: { start: "2026-10-05T09:00:00Z", end: "2026-10-05T10:00:00Z" } }), when)).toMatchObject({
      start: "2026-10-05T09:00:00Z",
      end: "2026-10-05T10:00:00Z",
      type: "range",
    });
  });

  it("makes a date range a range and a single date a point on the timeline", () => {
    expect(timelineItem(row({ due: "2026-10-05" }), field("due", "date"))).toMatchObject({
      id: "N1",
      content: "One",
      start: "2026-10-05",
      type: "point",
    });
    expect(
      timelineItem(row({ span: { start: "2026-10-05", end: "2026-10-07" } }), field("span", "date_range")),
    ).toMatchObject({ start: "2026-10-05", end: "2026-10-08", type: "range" });
    expect(timelineItem(row({ due: 5 }), field("due", "date"))).toBeNull();
  });
});

describe("visibleWindow", () => {
  it("compares days as local days and times as instants", () => {
    const start = new Date(2026, 9, 1);
    const end = new Date(2026, 10, 1);
    expect(visibleWindow(field("due", "date"), start, end)).toBe('due >= "2026-10-01" && due < "2026-11-01"');
    expect(visibleWindow(field("at", "datetime"), start, end)).toBe(
      `at >= "${start.toISOString()}" && at < "${end.toISOString()}"`,
    );
    expect(visibleWindow(field("span", "date_range", { of: "date" }), start, end)).toBe(
      'span.start < "2026-11-01" && span.end >= "2026-10-01"',
    );
    // Either form may be stored, so the window takes the notes either reading finds.
    expect(visibleWindow(field("when", "date_range", { of: "date_or_datetime" }), start, end)).toBe(
      '(when.start < "2026-11-01" && when.end >= "2026-10-01") || ' +
        `(when.start < "${end.toISOString()}" && when.end >= "${start.toISOString()}")`,
    );
  });
});
