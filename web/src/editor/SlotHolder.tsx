import { For, Show } from "solid-js";
import type { SolidNodeViewProps } from "./solidNodeView";

export interface SlotInfo {
  label: string;
  kind: string;
  /** False when the type has no field of this name. */
  known: boolean;
}

/** What a property block in the template editor needs from the editor around it. */
export interface SlotContext {
  describe: (name: string) => SlotInfo;
  /** The template's tab titles, and the tab being edited. */
  tabs: () => string[];
  tab: () => number;
  /** Move the property block at `pos` in this tab's editor to the end of tab `to`. */
  moveToTab: (pos: number, to: number) => void;
}

/**
 * A property block in the template editor: the field it places, a label to use instead of the
 * field's, and controls to move it to another tab or remove it. It is dragged like any block.
 */
export function SlotHolder(props: SolidNodeViewProps & { ctx: SlotContext }) {
  const slot = () => String(props.node().attrs.slot ?? "");
  const label = () => (props.node().attrs.label as string | null) ?? null;
  const info = () => props.ctx.describe(slot());
  return (
    <div
      class="slot-holder"
      classList={{ selected: props.selected(), unknown: !info().known }}
      data-testid={`template-item-${slot()}`}
    >
      <div class="slot-holder-label">
        <span class="slot-holder-icon" aria-hidden="true">
          ▤
        </span>
        <span class="slot-holder-name">{label() ?? info().label}</span>
        <span class="badge">{info().known ? `${info().kind} property` : "Unknown field"}</span>
      </div>
      <div class="slot-holder-controls">
        <input
          aria-label={`Label for ${slot()}`}
          placeholder={info().label}
          value={label() ?? ""}
          onInput={(e) => props.updateAttributes({ label: e.currentTarget.value || null })}
        />
        <Show when={props.ctx.tabs().length > 1}>
          <select
            aria-label={`Move ${slot()} to tab`}
            onChange={(e) => {
              const to = Number(e.currentTarget.value);
              const pos = props.getPos();
              if (to !== props.ctx.tab() && pos !== undefined) props.ctx.moveToTab(pos, to);
            }}
          >
            <For each={props.ctx.tabs()}>
              {(t, i) => (
                <option value={i()} selected={i() === props.ctx.tab()}>
                  {t || "Untitled tab"}
                </option>
              )}
            </For>
          </select>
        </Show>
        <button type="button" aria-label={`Remove ${slot()}`} onClick={() => props.replaceWith()}>
          ×
        </button>
      </div>
    </div>
  );
}
