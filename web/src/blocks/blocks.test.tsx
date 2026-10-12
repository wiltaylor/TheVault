import { fireEvent, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { describe, expect, it } from "vitest";
import type { Block } from "../api";
import { findLanguage, languageValue } from "./CodeBlock";
import { withItem, withText } from "./fields";
import { TerminalBlock } from "./TerminalBlock";

describe("block items", () => {
  it("drops an optional string item when it is emptied", () => {
    const b: Block = { kind: "image", fields: { asset: "A", alt: "x" } };
    expect(withText(b, "alt", "")).toEqual({ kind: "image", fields: { asset: "A" } });
    expect(withText(b, "caption", "c")).toEqual({ kind: "image", fields: { asset: "A", alt: "x", caption: "c" } });
    expect(withItem(b, "width", 640).fields?.width).toBe(640);
    expect(b.fields).toEqual({ asset: "A", alt: "x" });
  });
});

describe("code languages", () => {
  it("finds a language by name or alias in any case, and stores its name in lower case", () => {
    const rust = findLanguage("Rust");
    expect(rust?.name).toBe("Rust");
    expect(findLanguage("rust")).toBe(rust);
    expect(languageValue(rust!)).toBe("rust");
    expect(findLanguage("js")?.name).toBe("JavaScript");
    expect(findLanguage("")).toBeNull();
    expect(findLanguage("no-such-language")).toBeNull();
  });
});

describe("TerminalBlock", () => {
  const start: Block = {
    kind: "terminal",
    fields: { title: "Run" },
    blocks: [{ kind: "command", fields: { line: "ls", stdout: "a" } }],
  };

  it("edits commands in place and keeps each input mounted across edits", () => {
    const [block, setBlock] = createSignal(start);
    const { getByLabelText, getByText } = render(() => <TerminalBlock block={block()} onChange={setBlock} />);
    const line = getByLabelText("Command 1") as HTMLInputElement;
    fireEvent.input(line, { target: { value: "ls -l" } });
    expect(getByLabelText("Command 1")).toBe(line);
    fireEvent.input(getByLabelText("Output of command 1"), { target: { value: "" } });
    fireEvent.click(getByText("Add command"));
    fireEvent.input(getByLabelText("Errors of command 2"), { target: { value: "boom" } });
    fireEvent.input(getByLabelText("Exit code of command 2"), { target: { value: "3" } });
    expect(block()).toEqual({
      kind: "terminal",
      fields: { title: "Run" },
      blocks: [
        { kind: "command", fields: { line: "ls -l" } },
        { kind: "command", fields: { line: "", stderr: "boom", exit_code: 3 } },
      ],
    });
    fireEvent.click(getByLabelText("Remove command 1"));
    expect(block().blocks).toEqual([{ kind: "command", fields: { line: "", stderr: "boom", exit_code: 3 } }]);
  });

  it("renders stderr apart from stdout and a non-zero exit code", () => {
    const b: Block = {
      kind: "terminal",
      fields: { prompt: "%" },
      blocks: [{ kind: "command", fields: { line: "x", stdout: "out", stderr: "err", exit_code: 2 } }],
    };
    const { container } = render(() => <TerminalBlock block={b} />);
    expect(container.querySelector(".terminal-line")?.textContent).toBe("% x");
    expect(container.querySelector(".terminal-stdout")?.textContent).toBe("out");
    expect(container.querySelector(".terminal-stderr")?.textContent).toBe("err");
    expect(container.querySelector(".terminal-exit")?.textContent).toBe("exit 2");
  });
});
