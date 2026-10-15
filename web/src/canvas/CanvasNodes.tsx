// What each canvas node kind shows inside its box (docs/vault-format.md, "Canvas", D15): a card's
// content blocks, as a content field shows them, or a note through a template. A card being
// edited shows the block editor instead; CanvasSurface puts that in place.

import { Show, createResource } from "solid-js";
import { type Json, type Row, api } from "../api";
import { AssetCards } from "../blocks/MediaBlocks";
import { changes } from "../changes";
import { Blocks } from "../components/Render";
import { typeDef } from "../session";
import { TemplateBody } from "../views/parts";
import { viewTemplate } from "../views/model";
import { QueryPoll } from "../views/QueryView";
import type { CanvasNode } from "./canvas";

/** How often a query view in a card runs its query again, so notes made elsewhere appear. */
export const CARD_QUERY_POLL_MS = 3000;

const str = (v: Json | undefined) => (typeof v === "string" ? v : "");

function rowOf(id: string, type: string, fields: Record<string, Json>): Row {
  return { id, type, title: str(fields.title), fields };
}

/** A card's content blocks, read-only. */
function CardNode(props: { node: CanvasNode }) {
  return (
    <div class="canvas-card-blocks" data-testid="canvas-card">
      <Show when={(props.node.block.blocks ?? []).length > 0} fallback={<p class="muted canvas-card-empty">Empty card</p>}>
        <QueryPoll.Provider value={CARD_QUERY_POLL_MS}>
          {/* A card is too small for a PDF viewer: a PDF shows as its file card with a thumbnail. */}
          <AssetCards.Provider value={true}>
            <Blocks blocks={props.node.block.blocks ?? []} />
          </AssetCards.Provider>
        </QueryPoll.Provider>
      </Show>
    </div>
  );
}

/**
 * A note rendered with its `card` template, or the one the node names. Its title is not a link:
 * the node's Open button, a middle-click or a Ctrl/Cmd+click opens the note.
 */
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
              <strong class="query-title canvas-note-title">{row().title || "Untitled"}</strong>
              <TemplateBody row={row()} template={template()} doc={n().note} />
            </article>
          );
        }}
      </Show>
    </Show>
  );
}

export function NodeBody(props: { node: CanvasNode }) {
  return (
    <Show when={props.node.kind === "note"} fallback={<CardNode node={props.node} />}>
      <NoteNode node={props.node} />
    </Show>
  );
}
