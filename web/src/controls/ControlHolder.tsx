// A `control` block in the block editor: the control as the page shows it, and a small settings
// strip, shown while the block is hovered, focused or selected, that edits its label, its
// placeholder, its kind's own items and the field it binds (only fields it fits are offered).

import { For, Show, createSignal } from "solid-js";
import type { Block, FieldDef, Json } from "../api";
import { TITLE_FIELD } from "../notes";
import { useControlBinding } from "./binding";
import { ControlBlock } from "./ControlView";
import { CONTROL_INFO, RATING_MAX_DEFAULT, RATING_MAX_LIMIT, controlItems, controlOf, controlProblems, fieldKindText, fittingFields, withControlItem } from "./model";

export function ControlHolder(props: {
  block: Block;
  selected: boolean;
  /** Just inserted: the strip shows until the pointer or focus leaves. */
  fresh: boolean;
  onChange?: (block: Block) => void;
  onRemove?: () => void;
}) {
  const binding = useControlBinding();
  const [open, setOpen] = createSignal(props.fresh);
  const spec = () => controlOf(props.block);
  const fields = (): FieldDef[] => {
    const t = binding?.type();
    return t ? [TITLE_FIELD, ...t.fields] : [];
  };
  const bound = () => {
    const s = spec();
    return s ? fields().find((f) => f.name === s.field) : undefined;
  };
  const offered = () => {
    const s = spec();
    if (!s) return [];
    const fit = fittingFields(s.kind, fields());
    // The field it names stays listed even when it does not fit, so the choice shows what is there.
    const b = bound();
    return b && !fit.includes(b) ? [b, ...fit] : fit;
  };
  const set = (name: string, value: Json | undefined) => props.onChange?.(withControlItem(props.block, name, value));
  const numberItem = (name: string, raw: string) => {
    const t = raw.trim();
    const n = Number(t);
    set(name, t === "" || !Number.isFinite(n) ? undefined : n);
  };
  const problems = () => {
    const s = spec();
    return s ? controlProblems(s, bound()) : ["This control names no field or kind."];
  };
  const fieldText = (v: Json | undefined) => (typeof v === "number" ? String(v) : "");
  return (
    <div
      class="control-holder"
      classList={{ selected: props.selected, open: open(), invalid: problems().length > 0 }}
      onPointerLeave={() => setOpen(false)}
      onFocusOut={(e) => {
        if (!(e.relatedTarget instanceof Node && e.currentTarget.contains(e.relatedTarget))) setOpen(false);
      }}
    >
      <Show when={spec() && props.onChange}>
        {(_) => {
          const s = () => spec()!;
          const uses = () => controlItems(s().kind);
          return (
            <div class="control-strip" role="group" aria-label={`${CONTROL_INFO[s().kind].label} settings`} data-testid="control-strip" data-node-view-interactive>
              <span class="control-strip-kind">{CONTROL_INFO[s().kind].label}</span>
              <label>
                <span>Field</span>
                <select aria-label="Bound field" value={s().field} onChange={(e) => set("field", e.currentTarget.value)}>
                  <Show when={!bound()}>
                    <option value={s().field}>{s().field} (no such field)</option>
                  </Show>
                  <For each={offered()}>
                    {(f) => (
                      <option value={f.name} selected={f.name === s().field}>
                        {f.label} ({fieldKindText(f)})
                      </option>
                    )}
                  </For>
                </select>
              </label>
              <label>
                <span>Label</span>
                <input
                  aria-label="Control label"
                  placeholder={bound()?.label ?? s().field}
                  value={s().label ?? ""}
                  onInput={(e) => set("label", e.currentTarget.value || undefined)}
                />
              </label>
              <Show when={uses().includes("placeholder")}>
                <label>
                  <span>Placeholder</span>
                  <input aria-label="Placeholder" value={s().placeholder ?? ""} onInput={(e) => set("placeholder", e.currentTarget.value || undefined)} />
                </label>
              </Show>
              <Show when={uses().includes("min")}>
                <label class="control-strip-number">
                  <span>Min</span>
                  <input aria-label="Minimum" type="number" step="any" value={fieldText(props.block.fields?.min)} onChange={(e) => numberItem("min", e.currentTarget.value)} />
                </label>
                <label class="control-strip-number">
                  <span>Max</span>
                  <input aria-label="Maximum" type="number" step="any" value={fieldText(props.block.fields?.max)} onChange={(e) => numberItem("max", e.currentTarget.value)} />
                </label>
                <label class="control-strip-number">
                  <span>Step</span>
                  <input aria-label="Step" type="number" min="0" step="any" value={fieldText(props.block.fields?.step)} onChange={(e) => numberItem("step", e.currentTarget.value)} />
                </label>
              </Show>
              <Show when={s().kind === "rating"}>
                <label class="control-strip-number">
                  <span>Stars</span>
                  <input
                    aria-label="Stars"
                    type="number"
                    min="1"
                    max={RATING_MAX_LIMIT}
                    step="1"
                    placeholder={String(RATING_MAX_DEFAULT)}
                    value={fieldText(props.block.fields?.max)}
                    onChange={(e) => numberItem("max", e.currentTarget.value)}
                  />
                </label>
              </Show>
              <Show when={props.onRemove}>
                <button type="button" class="control-strip-remove" aria-label="Remove control" title="Remove control" onClick={() => props.onRemove?.()}>
                  ×
                </button>
              </Show>
              <Show when={problems().length > 0}>
                <p class="control-strip-problems" role="status">
                  {problems().join(" ")}
                </p>
              </Show>
            </div>
          );
        }}
      </Show>
      <ControlBlock block={props.block} />
    </div>
  );
}
