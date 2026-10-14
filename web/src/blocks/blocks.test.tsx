import { fireEvent, render, waitFor } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { describe, expect, it } from "vitest";
import type { Block } from "../api";
import { INSERTS } from "../editor/ContentEditor";
import { CodeBlock, findLanguage, isOutput, languageValue } from "./CodeBlock";
import { withItem, withText } from "./fields";

// jsdom lays nothing out; CodeMirror measures text ranges to draw the selection.
if (!Range.prototype.getClientRects) {
  Range.prototype.getClientRects = () => [] as unknown as DOMRectList;
  Range.prototype.getBoundingClientRect = () => new DOMRect();
}

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

  it("reads bash, sh and shell as the shell, stored as bash, and stdout as output, not a language", () => {
    const shell = findLanguage("bash");
    expect(shell?.name).toBe("Shell");
    expect(findLanguage("sh")).toBe(shell);
    expect(findLanguage("shell")).toBe(shell);
    expect(languageValue(shell!)).toBe("bash");
    expect(findLanguage("stdout")).toBeNull();
    expect(isOutput("stdout")).toBe(true);
    expect(isOutput("Stdout ")).toBe(true);
    expect(isOutput("bash")).toBe(false);
  });
});

describe("CodeBlock", () => {
  it("highlights a bash block as shell, labelled Bash", async () => {
    const b: Block = { kind: "code", fields: { language: "bash", source: "# list\necho \"hi\"\n" } };
    const { container } = render(() => <CodeBlock block={b} />);
    expect(container.querySelector(".code-language")?.textContent).toBe("Bash");
    await waitFor(() => expect(container.querySelector(".cm-content .tok-comment")?.textContent).toBe("# list"));
    expect(container.querySelector(".cm-content .tok-string")).not.toBeNull();
  });

  it("shows a stdout block as output: labelled Output, no highlighting and no line numbers, copy kept", async () => {
    const [block, setBlock] = createSignal<Block>({
      kind: "code",
      fields: { language: "stdout", source: "# not a comment\necho \"x\"\n" },
    });
    const { container, getByLabelText } = render(() => <CodeBlock block={block()} onChange={setBlock} />);
    expect(container.querySelector(".block-code")?.classList.contains("block-output")).toBe(true);
    expect(container.querySelector(".code-language")?.textContent).toBe("Output");
    expect(getByLabelText("Copy code")).not.toBeNull();
    expect(container.querySelector(".code-mirror.code-output")).not.toBeNull();
    // Give a language a chance to load: none does, and the editable view has no gutters.
    await new Promise((r) => setTimeout(r, 50));
    expect(container.querySelector(".cm-content [class*='tok-']")).toBeNull();
    expect(container.querySelector(".cm-gutters, .cm-lineNumbers")).toBeNull();
    expect(getByLabelText("Output").textContent).toContain("# not a comment");
  });

  it("offers Bash and Output in the language picker and stores bash and stdout", () => {
    const [block, setBlock] = createSignal<Block>({ kind: "code", fields: { source: "ls" } });
    const { container, getByLabelText } = render(() => <CodeBlock block={block()} onChange={setBlock} />);
    fireEvent.focusIn(container.querySelector(".block-code")!);
    const select = getByLabelText("Language") as HTMLSelectElement;
    const options = [...select.options].map((o) => o.textContent);
    expect(options).toContain("Output");
    expect(options).toContain("Bash");
    fireEvent.change(select, { target: { value: "Shell" } });
    expect(block().fields?.language).toBe("bash");
    fireEvent.change(getByLabelText("Language"), { target: { value: "stdout" } });
    expect(block().fields?.language).toBe("stdout");
    expect(container.querySelector(".code-mirror.code-output")).not.toBeNull();
  });
});

describe("slash menu inserts", () => {
  it("has Shell command and Output code blocks and no terminal", () => {
    const shell = INSERTS.find((i) => i.menu === "Shell command");
    const output = INSERTS.find((i) => i.menu === "Output");
    expect(shell?.block()).toEqual({ kind: "code", fields: { language: "bash", source: "" } });
    expect(output?.block()).toEqual({ kind: "code", fields: { language: "stdout", source: "" } });
    expect(INSERTS.some((i) => i.block().kind === "terminal")).toBe(false);
  });
});

