// A `table` content block, read-only (docs/vault-format.md, "table"): a real table, its header
// row and alignments as the block says, each cell's inline markup rendered, scrolling sideways
// inside the block when it is wider than the page. "Copy as Markdown" copies it as a GFM table.

import { For, Show, createSignal } from "solid-js";
import type { Block } from "../api";
import { copyText } from "../clipboard";
import { Inline } from "../components/Inline";
import { noteTitle } from "../editor/titles";
import { type Align, tableOf, tableToMarkdown } from "./table";

const alignStyle = (a: Align | undefined) => (a === "center" || a === "right" ? { "text-align": a } : undefined);

export function TableBlock(props: { block: Block }) {
  const t = () => tableOf(props.block);
  const body = () => (t().header ? t().rows.slice(1) : t().rows);
  const [state, setState] = createSignal<"idle" | "copied" | "failed">("idle");
  const copy = async () => {
    setState((await copyText(tableToMarkdown(t(), noteTitle))) ? "copied" : "failed");
    setTimeout(() => setState("idle"), 1500);
  };
  return (
    <div class="table-block" data-testid="table-block">
      <button type="button" class="table-copy table-copy-view" title="Copy as Markdown" onClick={() => void copy()}>
        {state() === "copied" ? "Copied" : state() === "failed" ? "Copy failed" : "Copy as Markdown"}
      </button>
      <div class="table-scroll">
        <table class="content-table">
          <Show when={t().header}>
            <thead>
              <tr>
                <For each={t().rows[0]}>
                  {(cell, c) => (
                    <th style={alignStyle(t().align[c()])}>
                      <Inline text={cell} />
                    </th>
                  )}
                </For>
              </tr>
            </thead>
          </Show>
          <tbody>
            <For each={body()}>
              {(row) => (
                <tr>
                  <For each={row}>
                    {(cell, c) => (
                      <td style={alignStyle(t().align[c()])}>
                        <Inline text={cell} />
                      </td>
                    )}
                  </For>
                </tr>
              )}
            </For>
          </tbody>
        </table>
      </div>
    </div>
  );
}
