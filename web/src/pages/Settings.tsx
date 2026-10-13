import { Show, createSignal } from "solid-js";
import { DEFAULT_TILES, isTileTemplate, setTileUrl, tileUrl, validTileUrl } from "../map/model";

/** Settings kept in this browser. */
export function Settings() {
  const [tiles, setTiles] = createSignal(tileUrl() === DEFAULT_TILES ? "" : tileUrl());
  const [saved, setSaved] = createSignal(false);
  const invalid = () => tiles().trim() !== "" && !validTileUrl(tiles().trim());

  function save(ev: SubmitEvent) {
    ev.preventDefault();
    if (invalid()) return;
    setTileUrl(tiles());
    setSaved(true);
  }

  return (
    <section class="settings">
      <h1>General settings</h1>
      <form class="settings-form" aria-label="Map tiles" onSubmit={save}>
        <h2>Map tiles</h2>
        <p class="muted">
          Real-world maps load their base map from here. Give a MapLibre style URL, or a raster tile URL with{" "}
          <code>{"{z}"}</code>, <code>{"{x}"}</code> and <code>{"{y}"}</code>. Leave it empty for OpenFreeMap. This
          setting is kept in this browser.
        </p>
        <label>
          Tile URL
          <input
            type="url"
            value={tiles()}
            placeholder={DEFAULT_TILES}
            aria-invalid={invalid()}
            onInput={(e) => {
              setTiles(e.currentTarget.value);
              setSaved(false);
            }}
          />
        </label>
        <Show when={invalid()}>
          <p class="error">The tile URL must be an absolute http or https URL.</p>
        </Show>
        <div class="settings-actions">
          <button type="submit" disabled={invalid()}>
            Save
          </button>
          <button
            type="button"
            onClick={() => {
              setTiles("");
              setTileUrl("");
              setSaved(true);
            }}
          >
            Use OpenFreeMap
          </button>
        </div>
        <Show when={saved()}>
          <p class="muted" role="status">
            Maps now use {isTileTemplate(tileUrl()) ? "raster tiles from" : "the style at"} {tileUrl()}.
          </p>
        </Show>
      </form>
    </section>
  );
}
