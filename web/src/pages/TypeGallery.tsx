import { A } from "@solidjs/router";
import { For, Show, createResource } from "solid-js";
import { type TypeDef, api } from "../api";
import { TypeIcon } from "../components/Icon";
import { types } from "../session";

/** How many notes of `name` the vault holds, from the total of a one-row query. */
async function noteCount(name: string): Promise<number> {
  return (await api.query(`from ${name}`, null, 1)).total;
}

function TypeCard(props: { type: TypeDef }) {
  const [count] = createResource(() => props.type.name, noteCount);
  const fields = () => props.type.fields.length;
  return (
    <li class="type-card" data-colour={props.type.colour ?? undefined}>
      <A href={`/settings/types/${props.type.name}/edit`} class="type-card-main">
        <span class="type-card-icon" aria-hidden="true">
          <TypeIcon name={props.type.icon} />
        </span>
        <span class="type-card-name">{props.type.plural}</span>
        <Show when={props.type.description}>
          <span class="type-card-description">{props.type.description}</span>
        </Show>
        <span class="type-card-meta">
          <Show when={!count.loading && !count.error} fallback={<span>… notes</span>}>
            <span data-testid="type-count">
              {count()} {count() === 1 ? "note" : "notes"}
            </span>
          </Show>
          <span>
            {fields()} {fields() === 1 ? "field" : "fields"}
          </span>
        </span>
      </A>
      <div class="type-card-actions">
        <A href={`/notes/new?type=${encodeURIComponent(props.type.name)}`}>New {props.type.label.toLowerCase()}</A>
        <A href={`/types/${props.type.name}`}>Notes</A>
        <A href={`/settings/types/${props.type.name}/templates`}>Templates</A>
      </div>
    </li>
  );
}

/** The vault's uploaded files, as a card beside the types. */
function AssetsCard() {
  const [assets] = createResource(() => api.assets());
  return (
    <li class="type-card type-card-assets">
      <A href="/assets" class="type-card-main">
        <span class="type-card-icon" aria-hidden="true">
          <TypeIcon name="image" />
        </span>
        <span class="type-card-name">Assets</span>
        <span class="type-card-description">Images and files uploaded to the vault.</span>
        <span class="type-card-meta">
          <Show when={!assets.loading && !assets.error} fallback={<span>… files</span>}>
            <span data-testid="asset-count">
              {assets()?.length} {assets()?.length === 1 ? "file" : "files"}
            </span>
          </Show>
        </span>
      </A>
    </li>
  );
}

/** Every type in the vault as a card that opens its editor, then the assets card and a card to create a new type. */
export function TypeGallery() {
  return (
    <section class="type-gallery-page">
      <header class="note-header">
        <h1>Types</h1>
      </header>
      <ul class="type-gallery" aria-label="Types">
        <For each={types()}>{(t) => <TypeCard type={t} />}</For>
        <AssetsCard />
        <li class="type-card type-card-new">
          <A href="/settings/types/new" class="type-card-main">
            <span class="type-card-icon" aria-hidden="true">
              +
            </span>
            <span class="type-card-name">New type</span>
            <span class="type-card-description">Define a schema, its fields and its templates.</span>
          </A>
        </li>
      </ul>
    </section>
  );
}
