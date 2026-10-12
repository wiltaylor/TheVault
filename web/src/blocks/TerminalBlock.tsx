// A recorded shell session: commands in order, each with its stdout, stderr and exit code. It is
// edited in place, in the terminal it renders as, so editing and reading look the same.

import { Index, Show } from "solid-js";
import type { Block } from "../api";
import { children, text, withItem, withText } from "./fields";

function exitCode(command: Block): number | undefined {
  const v = command.fields?.exit_code;
  return typeof v === "number" ? v : undefined;
}

/** A textarea as tall as its text, so output reads as it will render. */
function Output(props: { label: string; class: string; value: string; onInput: (v: string) => void }) {
  return (
    <textarea
      class={props.class}
      aria-label={props.label}
      placeholder={props.class === "terminal-stderr" ? "stderr" : "stdout"}
      rows={Math.max(1, props.value.split("\n").length)}
      spellcheck={false}
      value={props.value}
      onInput={(e) => props.onInput(e.currentTarget.value)}
    />
  );
}

export function TerminalBlock(props: { block: Block; onChange?: (block: Block) => void }) {
  const prompt = () => text(props.block, "prompt") || "$";
  const commands = () => children(props.block, "command");
  const change = (b: Block) => props.onChange?.(b);

  function setCommands(next: Block[]) {
    const rest = (props.block.blocks ?? []).filter((b) => b.kind !== "command");
    change({ ...props.block, blocks: [...rest, ...next] });
  }
  function setCommand(i: number, command: Block) {
    setCommands(commands().map((c, j) => (j === i ? command : c)));
  }

  return (
    <div class="block-terminal" classList={{ editing: !!props.onChange }}>
      <Show
        when={props.onChange}
        fallback={
          <Show when={text(props.block, "title") || text(props.block, "shell")}>
            <div class="terminal-bar">
              <span class="terminal-title">{text(props.block, "title")}</span>
              <span class="terminal-shell">{text(props.block, "shell")}</span>
            </div>
          </Show>
        }
      >
        <div class="terminal-bar">
          <input
            class="terminal-title"
            aria-label="Terminal title"
            placeholder="Title"
            value={text(props.block, "title")}
            onInput={(e) => change(withText(props.block, "title", e.currentTarget.value))}
          />
          <input
            class="terminal-shell"
            aria-label="Shell"
            placeholder="shell"
            value={text(props.block, "shell")}
            onInput={(e) => change(withText(props.block, "shell", e.currentTarget.value))}
          />
          <input
            class="terminal-prompt-input"
            aria-label="Prompt"
            placeholder="$"
            value={text(props.block, "prompt")}
            onInput={(e) => change(withText(props.block, "prompt", e.currentTarget.value))}
          />
        </div>
      </Show>
      <div class="terminal-body">
        {/* Index, not For: an edit makes a new command object, and the inputs must stay put. */}
        <Index each={commands()}>
          {(command, i) => (
            <div class="terminal-command">
              <Show
                when={props.onChange}
                fallback={
                  <>
                    <div class="terminal-line">
                      <span class="terminal-prompt">{prompt()}</span> {text(command(), "line")}
                    </div>
                    <Show when={text(command(), "stdout")}>
                      <div class="terminal-stdout">{text(command(), "stdout")}</div>
                    </Show>
                    <Show when={text(command(), "stderr")}>
                      <div class="terminal-stderr">{text(command(), "stderr")}</div>
                    </Show>
                    <Show when={(exitCode(command()) ?? 0) !== 0}>
                      <div class="terminal-exit">exit {exitCode(command())}</div>
                    </Show>
                  </>
                }
              >
                <div class="terminal-line">
                  <span class="terminal-prompt">{prompt()}</span>
                  <input
                    aria-label={`Command ${i + 1}`}
                    placeholder="command"
                    spellcheck={false}
                    value={text(command(), "line")}
                    onInput={(e) => setCommand(i, withItem(command(), "line", e.currentTarget.value))}
                    // Enter inside the note's form would submit it.
                    onKeyDown={(e) => e.key === "Enter" && e.preventDefault()}
                  />
                </div>
                <Output
                  label={`Output of command ${i + 1}`}
                  class="terminal-stdout"
                  value={text(command(), "stdout")}
                  onInput={(v) => setCommand(i, withText(command(), "stdout", v))}
                />
                <Output
                  label={`Errors of command ${i + 1}`}
                  class="terminal-stderr"
                  value={text(command(), "stderr")}
                  onInput={(v) => setCommand(i, withText(command(), "stderr", v))}
                />
                <div class="terminal-command-actions">
                  <label>
                    exit{" "}
                    <input
                      type="number"
                      step="1"
                      aria-label={`Exit code of command ${i + 1}`}
                      placeholder="0"
                      value={exitCode(command()) ?? ""}
                      onKeyDown={(e) => e.key === "Enter" && e.preventDefault()}
                      onInput={(e) => {
                        const v = e.currentTarget.value.trim();
                        const n = Number(v);
                        setCommand(i, withItem(command(), "exit_code", v === "" || !Number.isInteger(n) ? undefined : n));
                      }}
                    />
                  </label>
                  <Show when={commands().length > 1}>
                    <button
                      type="button"
                      aria-label={`Remove command ${i + 1}`}
                      onClick={() => setCommands(commands().filter((_, j) => j !== i))}
                    >
                      Remove
                    </button>
                  </Show>
                </div>
              </Show>
            </div>
          )}
        </Index>
        <Show when={props.onChange}>
          <button
            type="button"
            class="terminal-add"
            onClick={() => setCommands([...commands(), { kind: "command", fields: { line: "" } }])}
          >
            Add command
          </button>
        </Show>
      </div>
    </div>
  );
}
