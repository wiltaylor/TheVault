// Settings > Tools (script-tools T2): the list of script tools with their problems; the editor
// writing the tool file and its script in one PUT, a new tool's starter naming its own script;
// and the Run form, built from the tool's params, sending them in their kinds and showing the
// result, log, time and instructions, or why it failed.

import { Route, Router } from "@solidjs/router";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ScriptTool, ScriptToolSummary } from "../api";

const tools = vi.fn();
const tool = vi.fn();
const putTool = vi.fn();
const runTool = vi.fn();
const deleteTool = vi.fn();
vi.mock("../api", async (orig) => {
  const real = await orig<typeof import("../api")>();
  return { ...real, api: { ...real.api, tools, tool, putTool, runTool, deleteTool } };
});
// The script editor as a plain text box: CodeMirror does not run in the test DOM.
vi.mock("../blocks/LazyCode", () => ({
  CodeMirror: (p: { source: string; onChange?: (s: string) => void }) => (
    <textarea aria-label="Script source" value={p.source} onInput={(e) => p.onChange?.(e.currentTarget.value)} />
  ),
}));

const { ToolsList, ToolEditor, paramValues, scriptOf, starterWcl } = await import("./Tools");
const { ApiError } = await import("../api");

const SUMMARY: ScriptToolSummary = {
  name: "count_tagged",
  valid: true,
  problems: [],
  description: "Count the tasks with a tag.",
  script: "count_tagged",
  function: "run",
  params: [
    { name: "tag", kind: "string", description: "The tag.", required: true },
    { name: "limit", kind: "int", description: null, required: false },
    { name: "open", kind: "bool", required: false },
    { name: "names", kind: "list", of: "string", required: false },
    { name: "extra", kind: "json", required: false },
  ],
  allow_domains: [],
  secrets: [],
  timeout: 30,
};

const WCL = 'description = "Count the tasks with a tag."\nscript      = "count_tagged"\nfunction    = "run"\n';
const SOURCE = "fn run(params: Value) -> Result[Value, string] {\n    Ok(Value::Int(3))\n}\n";

function full(more: Partial<ScriptTool> = {}): ScriptTool {
  return { name: "count_tagged", wcl: WCL, valid: true, problems: [], tool: SUMMARY, script: "count_tagged", script_source: SOURCE, ...more };
}

beforeEach(() => {
  tools.mockResolvedValue([SUMMARY, { name: "broken", valid: false, problems: ["there is no script \"gone\""], script: "gone" }]);
  tool.mockResolvedValue(full());
  putTool.mockImplementation(async (_n: string, wcl: string, src?: string | null) => full({ wcl, script_source: src ?? SOURCE }));
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

function show(path: string) {
  window.history.replaceState({}, "", path);
  render(() => (
    <Router>
      <Route path="/settings/tools" component={ToolsList} />
      <Route path="/settings/tools/new" component={ToolEditor} />
      <Route path="/settings/tools/:name" component={ToolEditor} />
    </Router>
  ));
}

describe("the Tools list", () => {
  it("lists each tool with its description and script, and an invalid one with its problems", async () => {
    show("/settings/tools");
    const table = await screen.findByRole("table", { name: "Script tools" });
    const rows = within(table).getAllByRole("row");
    expect(rows).toHaveLength(3);
    expect(within(rows[1]!).getByRole("link", { name: "count_tagged" })).toHaveAttribute("href", "/settings/tools/count_tagged");
    expect(rows[1]).toHaveTextContent("Count the tasks with a tag.");
    expect(rows[2]).toHaveTextContent("not offered");
    expect(rows[2]).toHaveTextContent('there is no script "gone"');
    expect(screen.getByRole("link", { name: "New tool" })).toHaveAttribute("href", "/settings/tools/new");
  });
});

describe("the tool editor", () => {
  it("saves a changed script with the tool file in one PUT", async () => {
    show("/settings/tools/count_tagged");
    const box = (await screen.findByLabelText("Tool file")) as HTMLTextAreaElement;
    expect(box.value).toBe(WCL);
    fireEvent.input(box, { target: { value: WCL.replace("with a tag", "carrying a tag") } });
    fireEvent.input(screen.getByLabelText("Script source"), { target: { value: SOURCE.replace("3", "4") } });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(putTool).toHaveBeenCalled());
    expect(putTool).toHaveBeenCalledWith("count_tagged", WCL.replace("with a tag", "carrying a tag"), SOURCE.replace("3", "4"));
    expect(await screen.findByRole("status")).toHaveTextContent("Saved.");
  });

  it("leaves the script out of the PUT when only the tool file changed", async () => {
    show("/settings/tools/count_tagged");
    const box = await screen.findByLabelText("Tool file");
    fireEvent.input(box, { target: { value: `${WCL}timeout = 60\n` } });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(putTool).toHaveBeenCalled());
    expect(putTool.mock.calls[0]![2]).toBeNull();
  });

  it("shows the problems that keep a tool from being offered, and no Run form", async () => {
    tool.mockResolvedValue(full({ valid: false, problems: ["script \"count_tagged\" does not compile"] }));
    show("/settings/tools/count_tagged");
    expect(await screen.findByLabelText("Tool problems")).toHaveTextContent("does not compile");
    expect(screen.queryByRole("form", { name: "Run the tool" })).toBeNull();
  });

  it("shows why the server refused the file", async () => {
    putTool.mockRejectedValue(new ApiError(400, { error: "invalid", issues: [{ at: "description", message: "is required" }] }));
    show("/settings/tools/count_tagged");
    fireEvent.click(await screen.findByRole("button", { name: "Save" }));
    expect(await screen.findByText("Not saved: description: is required")).toBeInTheDocument();
  });

  it("starts a new tool naming a script of its own name, and opens it once saved", async () => {
    tool.mockResolvedValue(full({ name: "greet", wcl: starterWcl("greet"), script: "greet" }));
    show("/settings/tools/new");
    const name = await screen.findByLabelText("Name");
    fireEvent.input(name, { target: { value: "greet" } });
    expect((screen.getByLabelText("Tool file") as HTMLTextAreaElement).value).toBe(starterWcl("greet"));
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(putTool).toHaveBeenCalled());
    const [n, wcl, src] = putTool.mock.calls[0]!;
    expect(n).toBe("greet");
    expect(scriptOf(wcl as string)).toBe("greet");
    expect(src).toContain("fn run(params: Value)");
    await waitFor(() => expect(window.location.pathname).toBe("/settings/tools/greet"));
  });
});

describe("the Run form", () => {
  it("sends each param in its kind and shows the result, time, instructions and log", async () => {
    runTool.mockResolvedValue({ ok: true, result: { count: 3 }, log: "counted", duration_ms: 7, instructions: 120, script: { kind: "vault", hash: "ab" } });
    show("/settings/tools/count_tagged");
    const form = within(await screen.findByRole("form", { name: "Run the tool" }));
    fireEvent.input(form.getByLabelText("tag *"), { target: { value: "garden" } });
    fireEvent.input(form.getByLabelText("limit"), { target: { value: "5" } });
    fireEvent.click(form.getByLabelText("open"));
    fireEvent.input(form.getByLabelText("names"), { target: { value: "a\nb\n" } });
    fireEvent.input(form.getByLabelText("extra"), { target: { value: '{"x": 1}' } });
    fireEvent.click(form.getByRole("button", { name: "Run" }));
    await waitFor(() => expect(runTool).toHaveBeenCalled());
    expect(runTool).toHaveBeenCalledWith("count_tagged", { tag: "garden", limit: 5, open: true, names: ["a", "b"], extra: { x: 1 } });
    const result = await screen.findByTestId("tool-run-result");
    expect(result).toHaveTextContent("Succeeded");
    expect(result).toHaveTextContent("7 ms");
    expect(result).toHaveTextContent("120 instructions");
    expect(within(result).getByLabelText("Result")).toHaveTextContent('"count": 3');
    expect(within(result).getByLabelText("Log")).toHaveTextContent("counted");
  });

  it("shows a failed run with its kind and message", async () => {
    runTool.mockResolvedValue({ ok: false, error: "no tag", kind: "returned", log: "", duration_ms: 2, instructions: 10 });
    show("/settings/tools/count_tagged");
    const form = within(await screen.findByRole("form", { name: "Run the tool" }));
    fireEvent.input(form.getByLabelText("tag *"), { target: { value: "x" } });
    fireEvent.click(form.getByRole("button", { name: "Run" }));
    expect(await screen.findByTestId("tool-run-result")).toHaveTextContent("Failed (returned): no tag");
  });

  it("does not send a required param left empty", async () => {
    show("/settings/tools/count_tagged");
    const form = within(await screen.findByRole("form", { name: "Run the tool" }));
    fireEvent.click(form.getByRole("button", { name: "Run" }));
    expect(await form.findByRole("alert")).toHaveTextContent("tag is required.");
    expect(runTool).not.toHaveBeenCalled();
  });
});

describe("paramValues", () => {
  it("reads numbers, lists of ints and refuses what is not its kind", () => {
    const params = [
      { name: "n", kind: "float" as const, required: true },
      { name: "ids", kind: "list" as const, of: "int", required: false },
    ];
    expect(paramValues(params, { n: "1.5", ids: "1\n2" })).toEqual({ n: 1.5, ids: [1, 2] });
    expect(() => paramValues(params, { n: "x" })).toThrow("n must be a number.");
    expect(() => paramValues(params, { n: "1", ids: "a" })).toThrow("ids must be a whole number.");
  });
});
