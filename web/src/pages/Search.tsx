import { useSearchParams } from "@solidjs/router";
import { Show } from "solid-js";
import { api } from "../api";
import { PagedList } from "../components/PagedList";

/** Full-text search results, best match first. */
export function Search() {
  const [params] = useSearchParams();
  const q = () => (typeof params.q === "string" ? params.q.trim() : "");
  return (
    <section class="list-page">
      <h1>Search</h1>
      <Show when={q()} fallback={<p class="muted">Type in the search box to search every note.</p>}>
        <p class="muted">Results for “{q()}”</p>
        <PagedList
          key={q()}
          label="Search results"
          showType
          empty="No notes match."
          fetchPage={(cursor, size) => api.search(q(), cursor, size)}
        />
      </Show>
    </section>
  );
}
