// A download's card (ai D27): the file the AI asked to store in the vault as an asset, with its
// address, name, type and size, and Download and Discard. Once stored, by Apply or by the AI
// itself when Settings > AI lets downloads go ahead without asking, it names the new asset and
// shows an image's thumbnail from the asset's own URL.

import { Match, Show, Switch, createSignal } from "solid-js";
import { assetUrl } from "../api";
import { type Part, useChatSession } from "./store";

/** `n` bytes as people read a file size. */
export function fileSize(n: number | null | undefined): string {
  if (n === null || n === undefined) return "size unknown";
  if (n < 1024) return `${n} bytes`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / (1024 * 1024)).toFixed(1)} MB`;
}

export function DownloadCard(props: { part: Extract<Part, { kind: "download" }>; turn: number; index: number }) {
  const session = useChatSession();
  const p = () => props.part.proposal;
  const asset = () => props.part.asset;
  const name = () => asset()?.filename ?? p().filename;
  const media = () => asset()?.media_type ?? p().media_type ?? "";
  const open = () => props.part.state === "open" || props.part.state === "failed" || props.part.state === "applying";
  const browser = () => p().source === "browser";
  const label = () =>
    props.part.state === "applied"
      ? `${browser() ? "Saved" : "Downloaded"} ${name()}`
      : `Proposed ${browser() ? "save" : "download"} of ${name()}`;
  // In an app an asset not yet on the device does not load: its name is shown alone.
  const [broken, setBroken] = createSignal(false);
  return (
    <section class="ai-proposal" classList={{ "ai-applied-card": props.part.auto === true }} aria-label={label()}>
      <p class="ai-proposal-head" role={props.part.auto ? "status" : undefined}>
        <Show
          when={props.part.state === "applied" && asset()}
          fallback={
            <>
              Proposed {browser() ? "save" : "download"} of <strong>{name()}</strong>
            </>
          }
        >
          {(a) => (
            <>
              {browser() ? "Saved" : "Downloaded"}{" "}
              <a href={assetUrl(a().asset_id)} target="_blank" rel="noopener">
                {name()}
              </a>
            </>
          )}
        </Show>
      </p>
      <p class="muted">
        From{" "}
        <Show
          when={p().source !== "browser"}
          fallback={
            <span>
              the browser box: <code>{p().path}</code>
            </span>
          }
        >
          <Show when={p().url} fallback={<span>the web</span>}>
            <a href={p().url} target="_blank" rel="noopener noreferrer">
              {p().url}
            </a>
          </Show>
        </Show>
        {` (${[media(), fileSize(asset()?.size ?? p().size)].filter(Boolean).join(", ")})`}
      </p>
      <Show when={props.part.state === "applied" && asset() && media().startsWith("image/") && !broken() && asset()}>
        {(a) => (
          <a class="ai-step-thumb" href={assetUrl(a().asset_id)} target="_blank" rel="noopener">
            <img src={assetUrl(a().asset_id)} alt={name()} loading="lazy" onError={() => setBroken(true)} />
          </a>
        )}
      </Show>
      <Show when={props.part.error}>
        <p role="alert" class="ai-error">
          {props.part.error}
        </p>
      </Show>
      <Switch>
        <Match when={open()}>
          <div class="ai-proposal-actions">
            <button type="button" class="primary" disabled={props.part.state === "applying"} onClick={() => void session.applyDownload(props.turn, props.index)}>
              Apply
            </button>
            <button type="button" disabled={props.part.state === "applying"} onClick={() => session.discardDownload(props.turn, props.index)}>
              Discard
            </button>
          </div>
        </Match>
        <Match when={props.part.state === "applied"}>
          <p role="status" class={props.part.auto ? "muted" : "ai-applied"}>
            {props.part.auto ? "Downloaded without asking, as Settings > AI allows." : "Stored in the vault as an asset."}
          </p>
        </Match>
        <Match when={props.part.state === "discarded"}>
          <p role="status" class="muted">
            Discarded.
          </p>
        </Match>
      </Switch>
    </section>
  );
}
