import { A } from "@solidjs/router";
import { For, Show } from "solid-js";
import { types } from "../session";

export function Home() {
  return (
    <section class="home">
      <h1>Your vault</h1>
      <Show
        when={types().length > 0}
        fallback={<p class="muted">This vault has no types yet. Create one with New type, or through the API or the CLI.</p>}
      >
        <ul class="type-cards">
          <For each={types()}>
            {(t) => (
              <li>
                <A href={`/types/${t.name}`}>
                  <strong>{t.plural}</strong>
                  <Show when={t.description}>
                    <span class="muted">{t.description}</span>
                  </Show>
                </A>
              </li>
            )}
          </For>
        </ul>
      </Show>
    </section>
  );
}
