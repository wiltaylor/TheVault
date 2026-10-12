// What each canvas node kind shows inside its box (docs/vault-format.md, "Canvas", D15): text in
// inline markup, a note through a template, live query results, an asset, a link, or a drawing.

import { For, Match, Show, Switch, createEffect, createResource, createSignal, on, onCleanup, onMount } from "solid-js";
import { type Json, type Row, api, assetUrl } from "../api";
import { changes } from "../changes";
import { Inline } from "../components/Inline";
import { formatValue } from "../components/Render";
import { inkOf, paintOrder, strokePath } from "../ink/ink";
import { typeDef } from "../session";
import { TemplateBody } from "../views/parts";
import { viewTemplate } from "../views/model";
import { useThisNote } from "../views/thisNote";
import type { CanvasNode } from "./canvas";

/** Results a query node loads: a node is a fixed box, so more would never be seen. */
export const QUERY_NODE_ROWS = 100;

/** How often a query node runs its query again, so notes made elsewhere appear. */
export const QUERY_POLL_MS = 3000;

const str = (v: Json | undefined) => (typeof v === "string" ? v : "");

function TextNode(props: { node: CanvasNode }) {
  return (
    <div class="canvas-text">
      <Inline text={str(props.node.block.fields?.text)} />
    </div>
  );
}

/**
 * A result note's title as a link. A plain link, which the router picks up, because a canvas in
 * the block editor renders outside the router.
 */
function NoteLink(props: { row: Row }) {
  return (
    <a class="query-title" href={`/notes/${props.row.id}`} data-testid="query-title" draggable={false}>
      {props.row.title || "Untitled"}
    </a>
  );
}

function rowOf(id: string, type: string, fields: Record<string, Json>): Row {
  return { id, type, title: str(fields.title), fields };
}

/** A note rendered with its `card` template, or the one the node names. */
function NoteNode(props: { node: CanvasNode }) {
  const id = () => str(props.node.block.fields?.note);
  const [note] = createResource(
    () => [id(), changes()] as const,
    ([i]) => api.note(i),
  );
  return (
    <Show when={!note.error} fallback={<p class="muted">Missing note {id()}</p>}>
      <Show when={note.latest} fallback={<p class="muted">Loading…</p>}>
        {(n) => {
          const row = () => rowOf(n().id, n().type, n().note.fields);
          const template = () => viewTemplate(typeDef(n().type), str(props.node.block.fields?.template) || null, "card");
          return (
            <article class="canvas-card" data-testid="canvas-note">
              <NoteLink row={row()} />
              <TemplateBody row={row()} template={template()} doc={n().note} />
            </article>
          );
        }}
      </Show>
    </Show>
  );
}

/**
 * Live results of a query. The query runs again after every note write this client makes, when
 * the window regains focus, and every QUERY_POLL_MS, so a note made elsewhere shows up too.
 * `this` is the note that holds the canvas.
 */
function QueryNode(props: { node: CanvasNode }) {
  const thisId = useThisNote();
  const query = () => str(props.node.block.fields?.query);
  const view = () => str(props.node.block.fields?.view) || "cards";
  const [rows, setRows] = createSignal<Row[] | null>(null, {
    equals: (a, b) => JSON.stringify(a) === JSON.stringify(b),
  });
  const [total, setTotal] = createSignal(0);
  const [error, setError] = createSignal<string | null>(null);
  let run = 0;

  async function load() {
    const mine = ++run;
    try {
      const page = await api.query(query(), null, QUERY_NODE_ROWS, thisId);
      if (mine !== run) return;
      setRows(page.rows);
      setTotal(page.total);
      setError(null);
    } catch (e) {
      if (mine !== run) return;
      setError(e instanceof Error ? e.message : String(e));
    }
  }

  createEffect(on([query, changes], () => void load()));
  onMount(() => {
    const timer = setInterval(() => {
      if (document.visibilityState !== "hidden") void load();
    }, QUERY_POLL_MS);
    const onFocus = () => void load();
    window.addEventListener("focus", onFocus);
    onCleanup(() => {
      clearInterval(timer);
      window.removeEventListener("focus", onFocus);
    });
  });

  return (
    <div class="canvas-query" data-testid="canvas-query">
      <p class="canvas-query-head">
        <code>{query()}</code>
      </p>
      <Show when={error()}>
        <p class="error" role="alert">
          {error()}
        </p>
      </Show>
      <Show when={rows()} fallback={<Show when={!error()}><p class="muted">Loading…</p></Show>}>
        {(list) => (
          <>
            <p class="muted list-count" data-testid="canvas-query-count">
              {total() === 1 ? "1 note" : `${total()} notes`}
            </p>
            <Switch>
              <Match when={view() === "list"}>
                <ul class="canvas-query-list">
                  <For each={list()}>
                    {(row) => (
                      <li data-testid="query-row">
                        <NoteLink row={row} />
                      </li>
                    )}
                  </For>
                </ul>
              </Match>
              <Match when={view() === "table"}>
                <table class="canvas-query-table">
                  <tbody>
                    <For each={list()}>
                      {(row) => (
                        <tr data-testid="query-row">
                          <td>
                            <NoteLink row={row} />
                          </td>
                          <td class="muted">{typeDef(row.type)?.label ?? row.type}</td>
                          <td class="muted">{formatValue(row.fields.updated)}</td>
                        </tr>
                      )}
                    </For>
                  </tbody>
                </table>
              </Match>
              <Match when={true}>
                <div class="canvas-query-cards">
                  <For each={list()}>
                    {(row) => (
                      <article class="canvas-card" data-testid="query-card">
                        <NoteLink row={row} />
                        <TemplateBody
                          row={row}
                          template={viewTemplate(typeDef(row.type), str(props.node.block.fields?.template) || null, "card")}
                        />
                      </article>
                    )}
                  </For>
                </div>
              </Match>
            </Switch>
          </>
        )}
      </Show>
    </div>
  );
}

/** An image, or a card linking to the file when the asset is not one the browser can show. */
function AssetNode(props: { node: CanvasNode }) {
  const id = () => str(props.node.block.fields?.asset);
  const [broken, setBroken] = createSignal(false);
  createEffect(on(id, () => setBroken(false)));
  return (
    <Show
      when={!broken()}
      fallback={
        <a class="canvas-file" href={assetUrl(id())} target="_blank" rel="noopener noreferrer">
          File {id()}
        </a>
      }
    >
      <img class="canvas-image" src={assetUrl(id())} alt="" draggable={false} onError={() => setBroken(true)} />
    </Show>
  );
}

function LinkNode(props: { node: CanvasNode }) {
  const url = () => str(props.node.block.fields?.url);
  const host = () => {
    try {
      return new URL(url()).host;
    } catch {
      return url();
    }
  };
  return (
    <a class="canvas-link" href={url()} target="_blank" rel="noopener noreferrer" draggable={false}>
      <strong>{host()}</strong>
      <span class="muted">{url()}</span>
    </a>
  );
}

/** A drawing in the node's box: strokes in node-relative coordinates. */
function InkNode(props: { node: CanvasNode }) {
  const ink = () =>
    inkOf({ kind: "ink", fields: { width: props.node.width, height: props.node.height }, blocks: props.node.block.blocks });
  return (
    <svg class="canvas-ink" viewBox={`0 0 ${props.node.width} ${props.node.height}`} preserveAspectRatio="none">
      <For each={paintOrder(ink().strokes)}>
        {(s) => <path class={`ink-stroke ink-${s.stroke.tool}`} d={strokePath(s.stroke)} fill={s.stroke.colour} />}
      </For>
    </svg>
  );
}

export function NodeBody(props: { node: CanvasNode }) {
  return (
    <Switch fallback={<p class="muted">{props.node.kind} node</p>}>
      <Match when={props.node.kind === "text"}>
        <TextNode node={props.node} />
      </Match>
      <Match when={props.node.kind === "note"}>
        <NoteNode node={props.node} />
      </Match>
      <Match when={props.node.kind === "query"}>
        <QueryNode node={props.node} />
      </Match>
      <Match when={props.node.kind === "asset"}>
        <AssetNode node={props.node} />
      </Match>
      <Match when={props.node.kind === "link"}>
        <LinkNode node={props.node} />
      </Match>
      <Match when={props.node.kind === "ink"}>
        <InkNode node={props.node} />
      </Match>
    </Switch>
  );
}
