import { A, useParams } from "@solidjs/router";
import { Show } from "solid-js";
import { api } from "../api";
import { PagedList } from "../components/PagedList";
import { typeDef } from "../session";

/** Every note of one type, a page at a time, in a virtualised list (D33). */
export function TypeList() {
  const params = useParams();
  const name = () => params.type ?? "";
  return (
    <section class="list-page">
      <Show when={typeDef(name())} fallback={<p class="error">There is no type {name()}.</p>}>
        {(t) => (
          <>
            <header class="note-header">
              <h1>{t().plural}</h1>
              <div class="actions">
                <A href={`/notes/new?type=${encodeURIComponent(name())}`} class="button primary">
                  New {t().label.toLowerCase()}
                </A>
                <A href={`/settings/types/${name()}/edit`}>Edit type</A>
                <A href={`/settings/types/${name()}/templates`}>Templates</A>
              </div>
            </header>
            <PagedList
              key={name()}
              label={t().plural}
              empty={`No ${t().plural.toLowerCase()} yet.`}
              fetchPage={(cursor, size) => api.query(`from ${name()} sort title asc select title`, cursor, size)}
            />
          </>
        )}
      </Show>
    </section>
  );
}
