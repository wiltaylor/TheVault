import { fireEvent, render, waitFor, within } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { TypeDef } from "../api";
import { QueryEditor } from "./QueryEditor";
import { type QueryModel, numberOperand } from "./queryModel";

const EMPTY: QueryModel = { from: [], filter: null, search: null, sort: [], group_by: null, select: [], limit: null };

const TASK: TypeDef = {
  name: "task",
  label: "Task",
  plural: "Tasks",
  description: null,
  icon: null,
  colour: null,
  fields: [],
  templates: [],
};

/**
 * A stand-in for the two conversion endpoints. `models` answers `/api/query/parse`: a text it
 * holds has that model, any other text cannot be shown. `/api/query/text` records the model it
 * is sent and answers `text <n>`, or a 400 when the model has an empty field name.
 */
function server(models: Record<string, QueryModel>) {
  const sent: QueryModel[] = [];
  const json = (status: number, body: unknown) => new Response(JSON.stringify(body), { status });
  const fetch = vi.fn(async (url: string, init: RequestInit) => {
    const body = JSON.parse(String(init.body));
    if (url === "/api/query/parse") {
      const model = models[body.query] ?? null;
      return json(200, {
        text: body.query,
        model,
        builder_error: model ? null : "the builder cannot show this query: `*` is not a comparison",
      });
    }
    if (url === "/api/query/text") {
      sent.push(body.model);
      if (JSON.stringify(body.model).includes('"field":""')) {
        return json(400, { error: "invalid_model", message: 'the model is not a valid query: "" is not a name' });
      }
      return json(200, { text: `text ${sent.length}` });
    }
    return json(404, {});
  });
  vi.stubGlobal("fetch", fetch);
  return { fetch, sent };
}

function editor(start: string) {
  const [value, setValue] = createSignal(start);
  const onChange = vi.fn((t: string) => setValue(t));
  const r = render(() => <QueryEditor value={value()} onChange={onChange} types={[TASK]} />);
  return { ...r, value, onChange };
}

afterEach(() => vi.unstubAllGlobals());

/** Add a type in the builder's type picker by typing part of its name and picking it. */
async function addType(builder: HTMLElement, typed: string, label: string) {
  const b = within(builder);
  fireEvent.click(b.getByRole("button", { name: "Add to Types" }));
  const box = b.getByRole("combobox", { name: "Add to Types" }) as HTMLInputElement;
  box.value = typed;
  fireEvent.input(box);
  fireEvent.click(await b.findByRole("option", { name: label }));
}

describe("QueryEditor", () => {
  it("opens a query the builder can show in the builder, and never rewrites it unedited", async () => {
    // Written by an agent: not the canonical spacing, which the builder must not impose.
    const text = 'from   task where status=="todo"  sort title';
    const { fetch, sent } = server({
      [text]: {
        ...EMPTY,
        from: ["task"],
        filter: { join: "all", items: [{ condition: { field: "status", op: "eq", value: { kind: "str", value: "todo" } } }] },
        sort: [{ field: "title", order: "default" }],
      },
    });
    const { findByRole, getByRole, queryByLabelText, getByLabelText, value, onChange } = editor(text);
    const builder = await findByRole("group", { name: "Query builder" });
    expect(within(builder).getByLabelText("Field")).toHaveValue("status");
    expect(within(builder).getByLabelText("Value")).toHaveValue("todo");
    expect(within(builder).getByRole("button", { name: "Remove Task" })).toBeInTheDocument();
    expect(queryByLabelText("Query")).toBeNull();

    fireEvent.click(getByRole("button", { name: "Text" }));
    expect(getByLabelText("Query")).toHaveValue(text);
    fireEvent.click(getByRole("button", { name: "Builder" }));
    await findByRole("group", { name: "Query builder" });

    expect(onChange).not.toHaveBeenCalled();
    expect(sent).toEqual([]);
    expect(value()).toBe(text);
    expect(fetch.mock.calls.every(([url]) => url === "/api/query/parse")).toBe(true);
  });

  it("opens a query the builder cannot show as text, with a notice, and keeps it there", async () => {
    const text = "from task where estimate * 2 > 3";
    server({});
    const { findByTestId, getByRole, getByLabelText, queryByRole, onChange } = editor(text);
    expect(await findByTestId("builder-notice")).toHaveTextContent("the builder cannot show this query");
    expect(getByLabelText("Query")).toHaveValue(text);
    expect(getByRole("button", { name: "Text" })).toHaveAttribute("aria-pressed", "true");

    fireEvent.click(getByRole("button", { name: "Builder" }));
    await waitFor(() => expect(getByLabelText("Query")).toHaveValue(text));
    expect(queryByRole("group", { name: "Query builder" })).toBeNull();
    expect(onChange).not.toHaveBeenCalled();
  });

  it("builds a type, two conditions, a sort and a limit into one model", async () => {
    const { sent } = server({ "from *": EMPTY });
    const { findByRole, getByRole, getAllByTestId, value } = editor("from *");
    const builder = await findByRole("group", { name: "Query builder" });
    const b = within(builder);

    await addType(builder, "tas", "Task");
    fireEvent.click(b.getByRole("button", { name: "Add condition" }));
    let rows = getAllByTestId("condition");
    fireEvent.input(within(rows[0]!).getByLabelText("Field"), { target: { value: "status" } });
    fireEvent.input(within(rows[0]!).getByLabelText("Value"), { target: { value: "todo" } });

    fireEvent.click(b.getByRole("button", { name: "Add condition" }));
    rows = getAllByTestId("condition");
    const second = within(rows[1]!);
    fireEvent.input(second.getByLabelText("Field"), { target: { value: "estimate" } });
    fireEvent.change(second.getByLabelText("Operator"), { target: { value: "gt" } });
    fireEvent.change(second.getByLabelText("Value type"), { target: { value: "number" } });
    fireEvent.input(second.getByLabelText("Value"), { target: { value: "2" } });

    fireEvent.click(b.getByRole("button", { name: "Add sort" }));
    fireEvent.input(b.getByLabelText("Sort field"), { target: { value: "title" } });
    fireEvent.change(b.getByLabelText("Order"), { target: { value: "desc" } });
    fireEvent.input(b.getByLabelText("Limit"), { target: { value: "10" } });

    await waitFor(() => expect(value()).toBe(`text ${sent.length}`));
    expect(sent.at(-1)).toEqual({
      ...EMPTY,
      from: ["task"],
      filter: {
        join: "all",
        items: [
          { condition: { field: "status", op: "eq", value: { kind: "str", value: "todo" } } },
          { condition: { field: "estimate", op: "gt", value: { kind: "int", value: 2 } } },
        ],
      },
      sort: [{ field: "title", order: "desc" }],
      limit: 10,
    });
    expect(getByRole("button", { name: "Builder" })).toHaveAttribute("aria-pressed", "true");
  });

  it("keeps a builder edit made just before switching to text", async () => {
    server({ "from *": EMPTY });
    const { findByRole, getByRole, getByLabelText, value } = editor("from *");
    const builder = await findByRole("group", { name: "Query builder" });
    await addType(builder, "TASKS", "Task");
    fireEvent.click(getByRole("button", { name: "Text" }));
    await waitFor(() => expect(value()).toBe("text 1"));
    expect(getByLabelText("Query")).toHaveValue("text 1");
  });

  it("drops a builder edit's text that arrives after someone typed", async () => {
    const { fetch } = server({ "from *": EMPTY });
    const { findByRole, getByRole, getByLabelText, value } = editor("from *");
    const builder = await findByRole("group", { name: "Query builder" });
    await addType(builder, "TASKS", "Task");
    fireEvent.click(getByRole("button", { name: "Text" }));
    fireEvent.input(getByLabelText("Query"), { target: { value: "from page" } });
    await waitFor(() => expect(fetch.mock.calls.some(([url]) => url === "/api/query/text")).toBe(true));
    await new Promise((r) => setTimeout(r, 20));
    expect(value()).toBe("from page");
  });

  it("leaves the text alone while the model is not a query, and says why", async () => {
    server({ "from *": EMPTY });
    const { findByRole, findByRole: find, value, onChange } = editor("from *");
    const builder = await findByRole("group", { name: "Query builder" });
    fireEvent.click(within(builder).getByRole("button", { name: "Add condition" }));
    expect(await find("alert")).toHaveTextContent('"" is not a name');
    expect(onChange).not.toHaveBeenCalled();
    expect(value()).toBe("from *");
  });

  it("nests a group and removes it with its last condition", async () => {
    const { sent } = server({ "from *": EMPTY });
    const { findByRole, getAllByRole, getAllByTestId } = editor("from *");
    const builder = await findByRole("group", { name: "Query builder" });
    fireEvent.click(within(builder).getByRole("button", { name: "Add condition" }));
    fireEvent.click(getAllByRole("button", { name: "Add group" })[0]!);
    expect(getAllByTestId("condition-group")).toHaveLength(2);
    await waitFor(() =>
      expect(sent.at(-1)?.filter?.items).toEqual([
        { condition: { field: "", op: "eq", value: { kind: "str", value: "" } } },
        { group: { join: "any", items: [{ condition: { field: "", op: "eq", value: { kind: "str", value: "" } } }] } },
      ]),
    );
    fireEvent.click(getAllByRole("button", { name: "Remove condition" })[1]!);
    expect(getAllByTestId("condition-group")).toHaveLength(1);
  });
});

describe("numberOperand", () => {
  it("reads integers as ints, a point or exponent as a float, and anything else as nothing", () => {
    expect(numberOperand("2")).toEqual({ kind: "int", value: 2 });
    expect(numberOperand("-7")).toEqual({ kind: "int", value: -7 });
    expect(numberOperand("2.0")).toEqual({ kind: "float", value: 2 });
    expect(numberOperand("1e3")).toEqual({ kind: "float", value: 1000 });
    expect(numberOperand("")).toBeNull();
    expect(numberOperand("abc")).toBeNull();
    expect(numberOperand("99999999999999999999")).toBeNull();
  });
});
