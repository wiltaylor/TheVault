import { For, Show } from "solid-js";
import type { FieldDef, Json, NoteDoc, TypeDef } from "../api";
import { comparedFields, fieldValue, sameValue } from "../notes";
import { Blocks, formatValue } from "./Render";

export interface ConflictViewProps {
  type: TypeDef | undefined;
  mine: NoteDoc;
  stored: NoteDoc;
  busy: boolean;
  onKeepStored: () => void;
  onSaveMine: () => void;
}

function Value(props: { field: FieldDef; value: Json | undefined }) {
  return (
    <Show when={props.value !== undefined} fallback={<span class="muted">(empty)</span>}>
      <Show when={props.field.compound} fallback={formatValue(props.value)}>
        <Blocks blocks={((props.value as { blocks?: [] } | undefined)?.blocks ?? []) as []} />
      </Show>
    </Show>
  );
}

/**
 * Shown when a save answers 409: someone else saved the note since it was opened (D12). Your
 * version sits beside the stored one, field by field, with the differences marked.
 */
export function ConflictView(props: ConflictViewProps) {
  const fields = () => comparedFields(props.type, props.mine, props.stored);
  return (
    <section class="conflict" aria-labelledby="conflict-heading" data-testid="conflict-view">
      <h2 id="conflict-heading">This note changed while you were editing</h2>
      <p>
        Someone saved a newer version before you. Compare the two, then keep the stored version or
        save yours over it.
      </p>
      <div class="conflict-scroll">
        <table class="conflict-table">
          <thead>
            <tr>
              <th scope="col">Field</th>
              <th scope="col">Your version</th>
              <th scope="col">Stored version</th>
            </tr>
          </thead>
          <tbody>
            <For each={fields()}>
              {(f) => {
                const mine = () => fieldValue(props.mine, f);
                const stored = () => fieldValue(props.stored, f);
                const differs = () => !sameValue(mine(), stored());
                return (
                  <Show when={mine() !== undefined || stored() !== undefined}>
                    <tr classList={{ differs: differs() }} data-field={f.name}>
                      <th scope="row">
                        {f.label}
                        <Show when={differs()}>
                          <span class="badge">changed</span>
                        </Show>
                      </th>
                      <td data-testid="conflict-mine">
                        <Value field={f} value={mine()} />
                      </td>
                      <td data-testid="conflict-stored">
                        <Value field={f} value={stored()} />
                      </td>
                    </tr>
                  </Show>
                );
              }}
            </For>
          </tbody>
        </table>
      </div>
      <div class="actions">
        <button type="button" onClick={() => props.onKeepStored()} disabled={props.busy}>
          Keep the stored version
        </button>
        <button type="button" class="primary" onClick={() => props.onSaveMine()} disabled={props.busy}>
          Save my version
        </button>
      </div>
    </section>
  );
}
