import { describe, expect, it } from "vitest";
import type { FieldDef, TypeDef } from "../api";
import {
  EMPTY,
  completeOnly,
  type Filter,
  type Op,
  type SearchState,
  decodeSearch,
  describe as describeFilter,
  effectiveSort,
  encodeSearch,
  filterCond,
  filterCount,
  fieldGroups,
  lit,
  nextDay,
  opsFor,
  toQuery,
  withOp,
} from "./searchModel";

function field(name: string, kind: string, extra: Partial<FieldDef> = {}): FieldDef {
  return {
    name,
    label: name[0]!.toUpperCase() + name.slice(1),
    description: null,
    kind,
    required: false,
    computed: false,
    multiple: false,
    compound: false,
    ...extra,
  };
}

function type(name: string, fields: FieldDef[]): TypeDef {
  return { name, label: name, plural: `${name}s`, description: null, icon: null, colour: null, fields, templates: [] };
}

const TYPES: TypeDef[] = [
  type("task", [
    field("title", "string"),
    field("summary", "string"),
    field("link", "url"),
    field("estimate", "int"),
    field("score", "float"),
    field("done", "bool"),
    field("status", "select", { options: ["todo", "doing", "done"] }),
    field("labels", "select", { options: ["a", "b"], multiple: true }),
    field("stage", "taxonomy", { taxonomy: "stage" }),
    field("topics", "taxonomy", { taxonomy: "stage", multiple: true }),
    field("due", "date"),
    field("starts", "datetime"),
    field("span", "date_range"),
    field("window", "date_range", { of: "datetime" }),
    field("when", "date_range", { of: "date_or_datetime" }),
    field("project", "reference", { target: "project" }),
    field("people", "reference", { target: "person", multiple: true }),
    field("tags", "list", { items: "string" }),
    field("counts", "list", { items: "int" }),
    field("place", "location"),
    field("body", "content"),
  ]),
  type("project", [field("title", "string"), field("status", "select", { options: ["open"] }), field("due", "date")]),
];

const task = TYPES[0]!;
const def = (name: string) => task.fields.find((f) => f.name === name)!;
const UTC = () => 0;
const PLUS11 = () => 11 * 60;
const MINUS5 = () => -5 * 60;

function cond(name: string, op: Op, values: string[] = [], offset = UTC, today = () => "2026-10-20"): string | null {
  const f: Filter = { section: "prop", owner: "task", field: name, op, values };
  return filterCond(def(name), f, offset, today);
}

/**
 * Whether `when = { start, end }` meets condition `c` as the query engine reads it: each end by
 * its form, a date as its UTC midnight. Handles the comparisons, `&&`, `||` and parentheses the
 * Search page writes.
 */
function holds(c: string, start: string, end: string): boolean {
  const at = (v: string) => Date.parse(v.length === 10 ? `${v}T00:00:00Z` : v);
  const js = c
    .replace(/"([^"]*)"/g, (_, v: string) => String(at(v)))
    .replace(/when\.start/g, String(at(start)))
    .replace(/when\.end/g, String(at(end)));
  expect(js).toMatch(/^[\d\s<>=!&|()]+$/);
  return new Function(`return ${js};`)() as boolean;
}

describe("filter to condition", () => {
  it("text: contains, is, is not, empty", () => {
    expect(cond("summary", "contains", ["plan"])).toBe('contains(summary, "plan")');
    expect(cond("summary", "contains", ["  "])).toBeNull();
    expect(cond("summary", "is", ['say "hi" {x}'])).toBe('summary == "say \\"hi\\" {{x}}"');
    expect(cond("summary", "is_not", ["x"])).toBe('summary != "x"');
    expect(cond("link", "contains", ["example"])).toBe('contains(link, "example")');
    expect(cond("summary", "empty")).toBe("empty(summary)");
    expect(cond("summary", "not_empty")).toBe("!empty(summary)");
    expect(cond("summary", "is", [])).toBeNull();
  });

  it("number: every comparison, ints and floats, bad input left out", () => {
    expect(cond("estimate", "eq", ["3"])).toBe("estimate == 3");
    expect(cond("estimate", "ne", ["3"])).toBe("estimate != 3");
    expect(cond("estimate", "lt", ["3"])).toBe("estimate < 3");
    expect(cond("estimate", "le", ["-2"])).toBe("estimate <= -2");
    expect(cond("estimate", "gt", ["10"])).toBe("estimate > 10");
    expect(cond("score", "ge", ["2.5"])).toBe("score >= 2.5");
    expect(cond("estimate", "gt", ["abc"])).toBeNull();
    expect(cond("estimate", "gt", [""])).toBeNull();
    expect(cond("estimate", "empty")).toBe("empty(estimate)");
  });

  it("bool: true and false", () => {
    expect(cond("done", "true")).toBe("done");
    expect(cond("done", "false")).toBe("!done");
  });

  it("select: is, is not, any of, on one value and on several", () => {
    expect(cond("status", "is", ["todo"])).toBe('status == "todo"');
    expect(cond("status", "is_not", ["todo"])).toBe('status != "todo"');
    expect(cond("status", "any_of", ["todo", "doing"])).toBe('status == "todo" || status == "doing"');
    expect(cond("status", "any_of", ["todo"])).toBe('status == "todo"');
    expect(cond("status", "any_of", [])).toBeNull();
    expect(cond("labels", "is", ["a"])).toBe('has(labels, "a")');
    expect(cond("labels", "is_not", ["a"])).toBe('!has(labels, "a")');
    expect(cond("labels", "any_of", ["a", "b"])).toBe('has(labels, "a") || has(labels, "b")');
  });

  it("taxonomy: is, and under a term", () => {
    expect(cond("stage", "is", ["Active/Blocked"])).toBe('stage == "Active/Blocked"');
    expect(cond("stage", "under", ["Active"])).toBe('under(stage, "Active")');
    expect(cond("topics", "is", ["Tech"])).toBe('has(topics, "Tech")');
    expect(cond("topics", "under", ["Tech"])).toBe('under(topics, "Tech")');
  });

  it("reference: is a note, and is empty", () => {
    expect(cond("project", "is", ["01JB8Z4K2MQ7V3X9T5R6W8Y2P1"])).toBe('project == "01JB8Z4K2MQ7V3X9T5R6W8Y2P1"');
    expect(cond("people", "is", ["X"])).toBe('has(people, "X")');
    expect(cond("project", "empty")).toBe("empty(project)");
    expect(cond("project", "not_empty")).toBe("!empty(project)");
  });

  it("list: contains an item of the list's kind", () => {
    expect(cond("tags", "has", ["urgent"])).toBe('has(tags, "urgent")');
    expect(cond("counts", "has", ["4"])).toBe("has(counts, 4)");
    expect(cond("counts", "has", ["four"])).toBeNull();
    expect(cond("tags", "empty")).toBe("empty(tags)");
  });

  it("any kind: is empty and is not empty", () => {
    expect(cond("place", "empty")).toBe("empty(place)");
    expect(cond("place", "not_empty")).toBe("!empty(place)");
    expect(opsFor(def("place")).map(([o]) => o)).toEqual(["empty", "not_empty"]);
  });

  it("date: on, before, after, between and the last n days, as days", () => {
    expect(cond("due", "on", ["2026-10-14"])).toBe('due == "2026-10-14"');
    expect(cond("due", "before", ["2026-10-14"])).toBe('due < "2026-10-14"');
    expect(cond("due", "after", ["2026-10-14"])).toBe('due > "2026-10-14"');
    expect(cond("due", "between", ["2026-10-01", "2026-10-31"])).toBe('due >= "2026-10-01" && due <= "2026-10-31"');
    expect(cond("due", "between", ["2026-10-31", "2026-10-01"])).toBe('due >= "2026-10-01" && due <= "2026-10-31"');
    expect(cond("due", "between", ["2026-10-01"])).toBeNull();
    expect(cond("due", "last", ["7"])).toBe("due >= today() - days(7) && due <= today()");
    expect(cond("due", "last", ["0"])).toBeNull();
    expect(cond("due", "on", ["14/10/2026"])).toBeNull();
    expect(cond("due", "empty")).toBe("empty(due)");
  });

  it("datetime: days run from local midnight to the next", () => {
    expect(cond("starts", "on", ["2026-10-14"])).toBe('starts >= "2026-10-14T00:00:00Z" && starts < "2026-10-15T00:00:00Z"');
    expect(cond("starts", "on", ["2026-10-14"], PLUS11)).toBe(
      'starts >= "2026-10-14T00:00:00+11:00" && starts < "2026-10-15T00:00:00+11:00"',
    );
    expect(cond("starts", "before", ["2026-12-31"], MINUS5)).toBe('starts < "2026-12-31T00:00:00-05:00"');
    expect(cond("starts", "after", ["2026-12-31"])).toBe('starts >= "2027-01-01T00:00:00Z"');
    expect(cond("starts", "between", ["2026-10-01", "2026-10-31"])).toBe(
      'starts >= "2026-10-01T00:00:00Z" && starts < "2026-11-01T00:00:00Z"',
    );
    expect(cond("starts", "last", ["30"])).toBe("starts >= now() - days(30)");
  });

  it("date range: overlaps the days asked for", () => {
    expect(cond("span", "on", ["2026-10-14"])).toBe('span.start <= "2026-10-14" && span.end >= "2026-10-14"');
    expect(cond("span", "before", ["2026-10-14"])).toBe('span.start < "2026-10-14"');
    expect(cond("span", "after", ["2026-10-14"])).toBe('span.end > "2026-10-14"');
    expect(cond("span", "between", ["2026-10-01", "2026-10-07"])).toBe(
      'span.start <= "2026-10-07" && span.end >= "2026-10-01"',
    );
    expect(cond("span", "last", ["7"])).toBe("span.start <= today() && span.end >= today() - days(7)");
    expect(cond("window", "on", ["2026-10-14"])).toBe(
      'window.start < "2026-10-15T00:00:00Z" && window.end >= "2026-10-14T00:00:00Z"',
    );
    expect(cond("window", "last", ["2"])).toBe("window.start <= now() && window.end >= now() - days(2)");
  });

  it("date or datetime range: dated values as their whole local days, timed ones by instant", () => {
    const allDay = ["2026-10-20", "2026-10-20"] as const;
    const twoDays = ["2026-10-20", "2026-10-21"] as const;
    for (const offset of [UTC, PLUS11, MINUS5]) {
      const on = (d: string, [s, e]: readonly [string, string]) => holds(cond("when", "on", [d], offset)!, s, e);
      const when = (op: Op, vs: string[], [s, e]: readonly [string, string]) => holds(cond("when", op, vs, offset)!, s, e);
      expect([on("2026-10-19", allDay), on("2026-10-20", allDay), on("2026-10-21", allDay)]).toEqual([false, true, false]);
      expect([on("2026-10-19", twoDays), on("2026-10-21", twoDays), on("2026-10-22", twoDays)]).toEqual([false, true, false]);
      expect([when("before", ["2026-10-20"], allDay), when("before", ["2026-10-21"], allDay)]).toEqual([false, true]);
      expect([when("after", ["2026-10-19"], allDay), when("after", ["2026-10-20"], allDay)]).toEqual([true, false]);
      expect(when("between", ["2026-10-21", "2026-10-25"], allDay)).toBe(false);
      expect(when("between", ["2026-10-21", "2026-10-25"], twoDays)).toBe(true);
      expect([when("last", ["1"], allDay), when("last", ["1"], ["2026-10-18", "2026-10-18"])]).toEqual([true, false]);
    }
    // 19:30 to 20:30 on the 20th in New York is 00:30 to 01:30 UTC on the 21st.
    const evening = ["2026-10-20T19:30:00-05:00", "2026-10-20T20:30:00-05:00"] as const;
    const onIn = (d: string, offset: () => number, [s, e]: readonly [string, string]) =>
      holds(cond("when", "on", [d], offset)!, s, e);
    expect([onIn("2026-10-20", MINUS5, evening), onIn("2026-10-21", MINUS5, evening)]).toEqual([true, false]);
    expect(holds(cond("when", "before", ["2026-10-21"], MINUS5)!, ...evening)).toBe(true);
    expect(holds(cond("when", "after", ["2026-10-20"], MINUS5)!, ...evening)).toBe(false);
    // 08:00 on the 20th in Sydney is 21:00 UTC on the 19th.
    const morning = ["2026-10-20T08:00:00+11:00", "2026-10-20T09:00:00+11:00"] as const;
    expect([onIn("2026-10-19", PLUS11, morning), onIn("2026-10-20", PLUS11, morning)]).toEqual([false, true]);
    expect(holds(cond("when", "after", ["2026-10-19"], PLUS11)!, ...morning)).toBe(true);
    // The text: a bound at the local midnight, and west of UTC the UTC midnight a date sits on.
    expect(cond("when", "on", ["2026-10-20"], PLUS11)).toBe(
      'when.start < "2026-10-21T00:00:00+11:00" && when.end >= "2026-10-20T00:00:00+11:00"',
    );
    expect(cond("when", "on", ["2026-10-20"], MINUS5)).toBe(
      'when.start < "2026-10-21T00:00:00-05:00" && when.start != "2026-10-21" && ' +
        '(when.end >= "2026-10-20T00:00:00-05:00" || when.end == "2026-10-20")',
    );
    expect(cond("when", "last", ["0"])).toBeNull();
    expect(cond("when", "between", ["2026-10-01"])).toBeNull();
  });

  it("created and updated are instants and offer no emptiness test", () => {
    const f: Filter = { section: "date", owner: null, field: "created", op: "between", values: ["2026-01-01", "2026-01-31"] };
    const s: SearchState = { ...EMPTY, filters: [f] };
    expect(toQuery(s, TYPES, UTC)).toBe(
      'from * where created >= "2026-01-01T00:00:00Z" && created < "2026-02-01T00:00:00Z" sort updated desc',
    );
    expect(opsFor({ ...def("starts"), name: "updated" }).map(([o]) => o)).not.toContain("empty");
  });

  it("nextDay crosses months, years and leap days", () => {
    expect(nextDay("2026-01-31")).toBe("2026-02-01");
    expect(nextDay("2026-12-31")).toBe("2027-01-01");
    expect(nextDay("2028-02-28")).toBe("2028-02-29");
  });

  it("escapes strings as the query printer does", () => {
    expect(lit('a"b\\c\nd\t{}')).toBe('"a\\"b\\\\c\\nd\\t{{}}"');
    expect(lit("\u0001")).toBe('"\\u{1}"');
  });
});

describe("search to query text", () => {
  it("an empty search reads every note, newest first", () => {
    expect(toQuery(EMPTY, TYPES)).toBe("from * sort updated desc");
  });

  it("text searches by relevance unless another sort is chosen", () => {
    expect(toQuery({ ...EMPTY, text: " event sourcing" }, TYPES)).toBe('from * search "event sourcing"');
    expect(toQuery({ ...EMPTY, text: "x", sort: "title" }, TYPES)).toBe('from * search "x" sort title asc');
    expect(toQuery({ ...EMPTY, text: "x", sort: "created", dir: "asc" }, TYPES)).toBe(
      'from * search "x" sort created asc',
    );
    // Relevance with no text falls back to updated.
    expect(toQuery({ ...EMPTY, sort: "relevance" }, TYPES)).toBe("from * sort updated desc");
    expect(effectiveSort({ ...EMPTY, sort: "relevance" })).toEqual({ key: "updated", dir: "desc" });
  });

  it("keeps one trailing space, which makes the last word whole rather than a prefix", () => {
    expect(toQuery({ ...EMPTY, text: "meet" }, TYPES)).toBe('from * search "meet"');
    expect(toQuery({ ...EMPTY, text: " event sourcing  " }, TYPES)).toBe('from * search "event sourcing "');
    expect(toQuery({ ...EMPTY, text: "meet\t" }, TYPES)).toBe('from * search "meet "');
    expect(toQuery({ ...EMPTY, text: "   " }, TYPES)).toBe("from * sort updated desc");
  });

  it("types, filters, text and sort come in the grammar's order", () => {
    const s: SearchState = {
      text: "plan",
      types: ["task", "project"],
      filters: [
        { section: "prop", owner: null, field: "status", op: "any_of", values: ["todo", "doing"] },
        { section: "date", owner: null, field: "due", op: "before", values: ["2026-11-01"] },
        { section: "prop", owner: null, field: "estimate", op: "gt", values: [] },
      ],
      sort: "updated",
      dir: "asc",
      archived: "exclude",
    };
    expect(toQuery(s, TYPES)).toBe(
      'from task, project where (status == "todo" || status == "doing") && due < "2026-11-01" search "plan" sort updated asc',
    );
  });

  it("with no type chosen, a field's type narrows the query", () => {
    const one: SearchState = {
      ...EMPTY,
      filters: [{ section: "prop", owner: "task", field: "done", op: "true", values: [] }],
    };
    expect(toQuery(one, TYPES)).toBe("from task where done sort updated desc");
    const two: SearchState = {
      ...EMPTY,
      filters: [
        { section: "prop", owner: "task", field: "done", op: "false", values: [] },
        { section: "prop", owner: "project", field: "status", op: "is", values: ["open"] },
      ],
    };
    expect(toQuery(two, TYPES)).toBe(
      'from task, project where type == "task" && !done && type == "project" && status == "open" sort updated desc',
    );
  });

  it("a filter on a field no type has is left out", () => {
    const s: SearchState = {
      ...EMPTY,
      types: ["project"],
      filters: [{ section: "prop", owner: null, field: "estimate", op: "eq", values: ["1"] }],
    };
    expect(toQuery(s, TYPES)).toBe("from project sort updated desc");
  });

  it("leaves archived notes out unless the archived filter asks for them", () => {
    expect(toQuery({ ...EMPTY, archived: "include" }, TYPES)).toBe("from * include archived sort updated desc");
    expect(toQuery({ ...EMPTY, archived: "only" }, TYPES)).toBe("from * where archived sort updated desc");
    expect(toQuery({ ...EMPTY, text: "plan", types: ["task"], archived: "include" }, TYPES)).toBe(
      'from task include archived search "plan"',
    );
    const f: Filter = { section: "prop", owner: null, field: "done", op: "true", values: [] };
    expect(toQuery({ ...EMPTY, types: ["task"], filters: [f], archived: "only" }, TYPES)).toBe(
      "from task where done && archived sort updated desc",
    );
    expect(filterCount({ ...EMPTY, archived: "only" })).toBe(1);
    expect(filterCount({ ...EMPTY, archived: "exclude" })).toBe(0);
  });

  it("counts the type filter once", () => {
    expect(filterCount({ ...EMPTY, types: ["a", "b"], filters: [] })).toBe(1);
    expect(
      filterCount({ ...EMPTY, types: ["a"], filters: [{ section: "prop", owner: null, field: "x", op: "empty", values: [] }] }),
    ).toBe(2);
  });
});

describe("URL", () => {
  const full: SearchState = {
    text: 'a ~ b & c = "d" | e',
    types: ["task", "project"],
    filters: [
      { section: "date", owner: null, field: "created", op: "last", values: ["7"] },
      { section: "prop", owner: "task", field: "status", op: "any_of", values: ["to~do", "a|b", "x&y=z"] },
      { section: "date", owner: "task", field: "span", op: "between", values: ["2026-01-01", "2026-02-01"] },
      { section: "prop", owner: null, field: "project", op: "empty", values: [] },
      { section: "prop", owner: null, field: "summary", op: "is", values: [""] },
    ],
    sort: "title",
    dir: "desc",
    archived: "include",
  };

  it("round-trips every part of a search", () => {
    expect(decodeSearch(encodeSearch(full))).toEqual(full);
    expect(decodeSearch(`?${encodeSearch(full)}`)).toEqual(full);
  });

  it("the default search is an empty query string", () => {
    expect(encodeSearch(EMPTY)).toBe("");
    expect(decodeSearch("")).toEqual(EMPTY);
  });

  it("keeps the order filters were added in across sections", () => {
    const back = decodeSearch(encodeSearch(full));
    expect(back.filters.map((f) => f.field)).toEqual(["created", "status", "span", "project", "summary"]);
  });

  it("leaves filters' separators readable and escapes them inside values", () => {
    const qs = encodeSearch({ ...EMPTY, filters: [full.filters[1]!] });
    expect(qs).toBe("prop=task~status~any_of~to%7Edo~a%7Cb~x%26y%3Dz");
  });

  it("reads a plus as a space, as forms write it", () => {
    expect(decodeSearch("q=event+sourcing").text).toBe("event sourcing");
  });

  it("drops what it cannot read", () => {
    expect(decodeSearch("prop=x~y~nope&date=~~&sort=bogus&dir=up&type=").filters).toEqual([]);
    expect(decodeSearch("sort=bogus&dir=up")).toEqual(EMPTY);
    expect(decodeSearch("prop=%E0%A4%A")).toEqual(EMPTY);
  });

  it("reads like a short address", () => {
    expect(encodeSearch({ ...EMPTY, text: "zebra", types: ["task"] })).toBe("q=zebra&type=task");
  });

  it("writes the archived filter only when it asks for archived notes", () => {
    expect(encodeSearch({ ...EMPTY, archived: "exclude" })).toBe("");
    expect(encodeSearch({ ...EMPTY, text: "x", archived: "include" })).toBe("q=x&archived=include");
    expect(encodeSearch({ ...EMPTY, archived: "only", sort: "title" })).toBe("archived=only&sort=title");
    expect(decodeSearch("archived=only").archived).toBe("only");
    expect(decodeSearch("archived=include").archived).toBe("include");
    expect(decodeSearch("archived=exclude").archived).toBe("exclude");
    expect(decodeSearch("archived=maybe")).toEqual(EMPTY);
  });
});

describe("editing", () => {
  it("the address leaves out filters that are not complete yet", () => {
    const s: SearchState = {
      ...EMPTY,
      types: ["task"],
      filters: [
        { section: "prop", owner: "task", field: "status", op: "is", values: [] },
        { section: "prop", owner: "task", field: "status", op: "is", values: ["todo"] },
        { section: "prop", owner: "task", field: "project", op: "empty", values: [] },
        { section: "date", owner: "task", field: "due", op: "between", values: ["2026-01-01"] },
      ],
    };
    expect(completeOnly(s, TYPES).filters.map((f) => f.values)).toEqual([["todo"], []]);
    const done = { ...s, filters: s.filters.slice(1, 3) };
    expect(completeOnly(done, TYPES)).toBe(done);
  });

  it("changing an operator keeps the values that still fit", () => {
    const f: Filter = { section: "prop", owner: null, field: "status", op: "any_of", values: ["a", "b"] };
    expect(withOp(f, "is").values).toEqual(["a"]);
    expect(withOp(f, "empty").values).toEqual([]);
    const d: Filter = { section: "date", owner: null, field: "due", op: "between", values: ["2026-01-01", "2026-01-02"] };
    expect(withOp(d, "on").values).toEqual(["2026-01-01"]);
    expect(withOp(d, "last").values).toEqual(["7"]);
    expect(withOp({ ...d, op: "last", values: ["3"] }, "on").values).toEqual([]);
  });

  it("offers fields of the chosen types once, or of every type grouped by type", () => {
    const want = (f: FieldDef) => f.kind === "date";
    expect(fieldGroups(TYPES, ["task", "project"], want)).toEqual([{ type: null, fields: [def("due")] }]);
    const grouped = fieldGroups(TYPES, [], want);
    expect(grouped.map((g) => [g.type?.name, g.fields.map((f) => f.name)])).toEqual([
      ["task", ["due"]],
      ["project", ["due"]],
    ]);
  });

  it("describes a filter for its chip", () => {
    const f = (op: Op, values: string[]): Filter => ({ section: "prop", owner: null, field: "due", op, values });
    expect(describeFilter(def("due"), f("last", ["7"]))).toBe("Due in the last 7 days");
    expect(describeFilter(def("due"), f("between", ["2026-01-01", "2026-01-31"]))).toBe(
      "Due between 2026-01-01 and 2026-01-31",
    );
    expect(describeFilter(def("due"), f("empty", []))).toBe("Due is empty");
    expect(describeFilter(def("status"), { ...f("any_of", ["a", "b"]), field: "status" })).toBe("Status is any of a, b");
  });
});
