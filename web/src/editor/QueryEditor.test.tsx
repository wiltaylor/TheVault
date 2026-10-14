import { fireEvent, render, waitFor } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";
import { QueryEditor } from "./QueryEditor";

/** A stand-in for `/api/query/parse`: a query holding `@@` is a syntax error, any other parses. */
function server() {
  const json = (status: number, body: unknown) => new Response(JSON.stringify(body), { status });
  const fetch = vi.fn(async (_url: string, init: RequestInit) => {
    const query = String(JSON.parse(String(init.body)).query);
    return query.includes("@@")
      ? json(400, { error: "query_syntax", message: "line 1, column 6: unexpected `@`" })
      : json(200, { text: query });
  });
  vi.stubGlobal("fetch", fetch);
  return fetch;
}

function editor(start: string) {
  const [value, setValue] = createSignal(start);
  const onChange = vi.fn((t: string) => setValue(t));
  const r = render(() => <QueryEditor value={value()} onChange={onChange} />);
  const box = () => r.getByRole("textbox", { name: "Query" }) as HTMLTextAreaElement;
  const type = (text: string) => {
    box().value = text;
    fireEvent.input(box());
  };
  return { ...r, value, onChange, box, type };
}

afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

describe("QueryEditor", () => {
  it("is a text box with no builder", () => {
    server();
    const { box, queryByRole } = editor("from task");
    expect(box().value).toBe("from task");
    expect(queryByRole("button", { name: "Builder" })).toBeNull();
    expect(queryByRole("button", { name: "Text" })).toBeNull();
  });

  it("passes typed text straight through", () => {
    server();
    const { type, onChange, value } = editor("");
    type("from task sort due");
    expect(onChange).toHaveBeenLastCalledWith("from task sort due");
    expect(value()).toBe("from task sort due");
  });

  it("shows a syntax error once typing stops, and clears it when the query parses", async () => {
    const fetch = server();
    const { type, findByRole, queryByRole, box } = editor("from task");
    type("from @@");
    expect(await findByRole("alert")).toHaveTextContent("line 1, column 6: unexpected `@`");
    expect(box()).toHaveAttribute("aria-invalid", "true");
    type("from task");
    await waitFor(() => expect(queryByRole("alert")).toBeNull());
    expect(box()).toHaveAttribute("aria-invalid", "false");
    expect(fetch).toHaveBeenCalled();
  });

  it("checks only the text typing stopped on", async () => {
    vi.useFakeTimers();
    const fetch = server();
    const { type } = editor("");
    type("f");
    type("fr");
    type("from task");
    await vi.advanceTimersByTimeAsync(1000);
    expect(fetch).toHaveBeenCalledTimes(1);
    expect(JSON.parse(String(fetch.mock.calls[0]![1].body)).query).toBe("from task");
  });

  it("does not check an empty query", async () => {
    vi.useFakeTimers();
    const fetch = server();
    editor("   ");
    await vi.advanceTimersByTimeAsync(1000);
    expect(fetch).not.toHaveBeenCalled();
  });
});
