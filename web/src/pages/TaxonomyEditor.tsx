import { A, useNavigate, useParams } from "@solidjs/router";
import { For, Show, createEffect, createMemo, createSignal, on, onCleanup } from "solid-js";
import { createStore, produce, reconcile } from "solid-js/store";
import { ApiError, type UsingNote, api, usingNotes } from "../api";
import { colourLabel, paletteCss } from "../canvas/palette";
import { UsingNotes } from "../components/UsingNotes";
import {
  type TaxonomyModel,
  type TermModel,
  locate,
  newTerm,
  problems,
  taxonomyFromSchema,
  taxonomyWcl,
  termColours,
  termRenames,
} from "../defs/taxonomyModel";
import { isName, PALETTE } from "../defs/typeModel";
import { reloadDefinitions, taxonomies } from "../session";
import { errorText } from "./TypeEditor";

/** Every taxonomy, and a way to start a new one (D7). */
export function TaxonomyList() {
  return (
    <section class="list-page">
      <header class="note-header">
        <h1>Taxonomies</h1>
        <A href="/settings/taxonomies/new" class="button">
          New taxonomy
        </A>
      </header>
      <Show when={taxonomies().length > 0} fallback={<p class="muted">No taxonomies yet.</p>}>
        <ul class="type-cards">
          <For each={taxonomies()}>
            {(t) => (
              <li>
                <A href={`/settings/taxonomies/${t.name}`}>
                  <strong>{t.label}</strong>
                  <span class="muted">{t.description ?? t.name}</span>
                </A>
              </li>
            )}
          </For>
        </ul>
      </Show>
    </section>
  );
}

/**
 * The taxonomy editor (D7): a term tree whose terms are added, renamed, reordered, nested and
 * removed in place. Saving rewrites every note holding a renamed or moved term in the same commit.
 */
export function TaxonomyEditor() {
  const params = useParams();
  const navigate = useNavigate();
  const isNew = () => params.name === undefined;
  const [name, setName] = createSignal("");
  const [model, setModel] = createStore<TaxonomyModel>({ label: "", description: "", allow_new: false, terms: [] });
  const [loaded, setLoaded] = createSignal(false);
  const [loadError, setLoadError] = createSignal<string | null>(null);
  const [error, setError] = createSignal<string | null>(null);
  const [status, setStatus] = createSignal<string | null>(null);
  const [busy, setBusy] = createSignal(false);
  // The notes named by a refused save or delete.
  const [using, setUsing] = createSignal<UsingNote[]>([]);

  createEffect(
    on(
      () => params.name,
      async (n) => {
        setLoaded(false);
        setLoadError(null);
        setError(null);
        setStatus(null);
        setUsing([]);
        if (n === undefined) {
          setName("");
          setModel(reconcile({ label: "", description: "", allow_new: false, terms: [newTerm("First term")] }));
          setLoaded(true);
          return;
        }
        try {
          const t = (await api.taxonomies()).find((x) => x.name === n);
          if (!t) {
            setLoadError(`There is no taxonomy ${n}.`);
            return;
          }
          setName(n);
          setModel(reconcile(taxonomyFromSchema(t)));
          setLoaded(true);
        } catch (e) {
          setLoadError(errorText(e));
        }
      },
    ),
  );

  function edit(fn: (m: TaxonomyModel) => void) {
    setModel(produce(fn));
    setError(null);
    setStatus(null);
  }

  /** Apply `fn` to the term `key` where it sits in the tree. */
  function at(key: number, fn: (where: NonNullable<ReturnType<typeof locate>>) => void) {
    edit((m) => {
      const where = locate(m.terms, key);
      if (where) fn(where);
    });
  }

  const moveBy = (key: number, by: number) =>
    at(key, ({ list, index }) => {
      const j = index + by;
      if (j < 0 || j >= list.length) return;
      const [t] = list.splice(index, 1);
      list.splice(j, 0, t!);
    });

  /** Make the term a child of the sibling above it. */
  const indent = (key: number) =>
    at(key, ({ list, index }) => {
      const above = list[index - 1];
      if (!above) return;
      const [t] = list.splice(index, 1);
      above.children.push(t!);
    });

  /** Make the term a sibling following its parent. */
  const outdent = (key: number) =>
    at(key, ({ list, index, parent, parentList }) => {
      if (!parent || !parentList) return;
      const [t] = list.splice(index, 1);
      parentList.splice(parentList.indexOf(parent) + 1, 0, t!);
    });

  // Every term's colour, own or inherited, as edited so far.
  const colours = createMemo(() => termColours(model.terms));
  // The term whose palette is open, by key.
  const [palette, setPalette] = createSignal<number | null>(null);
  const closeOutside = (e: PointerEvent) => {
    if (!(e.target instanceof Element) || !e.target.closest(".term-colour")) setPalette(null);
  };
  window.addEventListener("pointerdown", closeOutside, true);
  onCleanup(() => window.removeEventListener("pointerdown", closeOutside, true));

  const issues = () => [...(isNew() && !isName(name()) ? ["The taxonomy needs a name: a-z, 0-9 and _."] : []), ...problems(model)];

  async function save() {
    setBusy(true);
    setError(null);
    setStatus(null);
    setUsing([]);
    try {
      const renames = termRenames(model);
      const saved = await api.putTaxonomy(name(), taxonomyWcl(model), renames);
      await reloadDefinitions();
      if (isNew()) {
        navigate(`/settings/taxonomies/${name()}`);
        return;
      }
      setModel(reconcile(taxonomyFromSchema(saved)));
      const n = saved.notes?.length ?? 0;
      setStatus(n ? `Saved. ${n} ${n === 1 ? "note was" : "notes were"} updated.` : "Saved.");
    } catch (e) {
      if (e instanceof ApiError && e.body.error === "term_in_use") {
        setError("Notes still hold terms this change removes. Keep those terms, or rename them, and save again.");
        setUsing(usingNotes(e));
      } else {
        setError(errorText(e));
      }
    } finally {
      setBusy(false);
    }
  }

  async function remove() {
    if (!confirm(`Delete taxonomy ${name()}?`)) return;
    setBusy(true);
    setError(null);
    setStatus(null);
    setUsing([]);
    try {
      await api.deleteTaxonomy(name());
      await reloadDefinitions();
      navigate("/settings/taxonomies");
    } catch (e) {
      if (e instanceof ApiError && e.body.error === "taxonomy_in_use") {
        const by = Array.isArray(e.body.by) ? (e.body.by as string[]) : [];
        const fields = by.flatMap((b) => {
          const m = /^types\/([^/]+)\.wcl \(field (\w+)\)$/.exec(b);
          return m ? [`${m[1]}.${m[2]}`] : [];
        });
        const notes = usingNotes(e);
        setError(
          `${model.label || name()} is in use, so it was not deleted. ` +
            (fields.length ? `Fields that use it: ${fields.join(", ")}. ` : "") +
            (notes.length ? `${notes.length} ${notes.length === 1 ? "note holds" : "notes hold"} its terms.` : "No note holds its terms."),
        );
        setUsing(notes);
      } else {
        setError(errorText(e));
      }
    } finally {
      setBusy(false);
    }
  }

  /**
   * A term's colour: a swatch that opens the palette. A term with no colour of its own shows the
   * colour it inherits, as a ring, and "Inherit" in the palette clears its own.
   */
  function TermColour(props: { term: TermModel; path: string }) {
    const key = props.term.key;
    const own = () => props.term.colour;
    const inherited = () => (own() ? undefined : parentColour(props.path));
    const open = () => palette() === key;
    let button!: HTMLButtonElement;
    const choose = (c: string) => {
      at(key, ({ list, index }) => (list[index]!.colour = c));
      setPalette(null);
      button.focus();
    };
    const title = () =>
      own() ? colourLabel(own()) : inherited() ? `Inherits ${colourLabel(inherited()).toLowerCase()}` : "No colour";
    return (
      <span class="term-colour">
        <button
          type="button"
          ref={button}
          class="term-colour-button"
          aria-label={`Colour of ${props.path}`}
          title={title()}
          aria-haspopup="true"
          aria-expanded={open()}
          data-colour={own() || inherited() || undefined}
          data-inherited={inherited() ? "" : undefined}
          onClick={() => setPalette(open() ? null : key)}
        >
          <span
            class="term-dot"
            classList={{ none: !own() && !inherited(), inherited: !!inherited() }}
            style={{ "--dot": paletteCss(own() || inherited()) }}
          />
        </button>
        <Show when={open()}>
          <div
            class="term-palette"
            role="group"
            aria-label={`Colours for ${props.path}`}
            onKeyDown={(e) => {
              if (e.key === "Escape") {
                e.preventDefault();
                e.stopPropagation();
                setPalette(null);
                button.focus();
              }
            }}
          >
            <button
              type="button"
              class="term-palette-item"
              aria-label={parentColour(props.path) ? "Inherit" : "No colour"}
              title={parentColour(props.path) ? `Inherit ${parentColour(props.path)}` : "No colour"}
              aria-pressed={!own()}
              onClick={() => choose("")}
              ref={(el) => queueMicrotask(() => el.focus({ preventScroll: true }))}
            >
              <span class="term-dot none" />
            </button>
            <For each={PALETTE}>
              {(c) => (
                <button
                  type="button"
                  class="term-palette-item"
                  aria-label={colourLabel(c)}
                  title={colourLabel(c)}
                  aria-pressed={own() === c}
                  data-colour={c}
                  onClick={() => choose(c)}
                >
                  <span class="term-dot" style={{ "--dot": paletteCss(c) }} />
                </button>
              )}
            </For>
          </div>
        </Show>
      </span>
    );
  }

  /** The colour a term takes with none of its own: its parent's, own or inherited. */
  const parentColour = (path: string) => (path.includes("/") ? colours().get(path.slice(0, path.lastIndexOf("/"))) : undefined);

  function TermRow(props: { term: TermModel; path: string; first: boolean; last: boolean; nested: boolean }) {
    const t = () => props.term;
    const key = t().key;
    return (
      <li class="term" data-testid={`term-${props.path}`}>
        <div class="term-row">
          <TermColour term={t()} path={props.path} />
          <input
            aria-label={`Name of ${props.path}`}
            value={t().name}
            onInput={(e) => at(key, ({ list, index }) => (list[index]!.name = e.currentTarget.value))}
          />
          <Show when={!t().colour && parentColour(props.path)}>
            {(c) => <span class="term-inherits muted">inherits {c()}</span>}
          </Show>
          <Show when={t().original !== null && t().original !== props.path}>
            <span class="muted">was {t().original}</span>
          </Show>
          <button type="button" aria-label={`Add a term under ${props.path}`} onClick={() => at(key, ({ list, index }) => void list[index]!.children.push(newTerm("New term")))}>
            + child
          </button>
          <button type="button" aria-label={`Move ${props.path} up`} disabled={props.first} onClick={() => moveBy(key, -1)}>
            ↑
          </button>
          <button type="button" aria-label={`Move ${props.path} down`} disabled={props.last} onClick={() => moveBy(key, 1)}>
            ↓
          </button>
          <button type="button" aria-label={`Nest ${props.path} under the term above`} disabled={props.first} onClick={() => indent(key)}>
            →
          </button>
          <button type="button" aria-label={`Move ${props.path} out a level`} disabled={!props.nested} onClick={() => outdent(key)}>
            ←
          </button>
          <button type="button" aria-label={`Remove ${props.path}`} onClick={() => at(key, ({ list, index }) => void list.splice(index, 1))}>
            ×
          </button>
        </div>
        <Show when={t().children.length > 0}>
          <Terms terms={t().children} prefix={props.path} />
        </Show>
      </li>
    );
  }

  function Terms(props: { terms: TermModel[]; prefix: string }) {
    return (
      <ul class="term-tree" role={props.prefix ? "group" : "tree"} aria-label={props.prefix ? undefined : "Terms"}>
        <For each={props.terms}>
          {(t, i) => (
            <TermRow
              term={t}
              path={props.prefix ? `${props.prefix}/${t.name}` : t.name}
              first={i() === 0}
              last={i() === props.terms.length - 1}
              nested={props.prefix !== ""}
            />
          )}
        </For>
      </ul>
    );
  }

  return (
    <section class="def-editor">
      <Show when={loadError()}>
        <p class="error" role="alert">
          {loadError()}
        </p>
      </Show>
      <Show when={loaded()}>
        <header class="note-header">
          <div>
            <p class="note-kind">{isNew() ? "New taxonomy" : "Taxonomy"}</p>
            <h1>{model.label || name() || "Untitled taxonomy"}</h1>
          </div>
          <Show when={!isNew()}>
            <div class="actions">
              <button type="button" class="danger" disabled={busy()} onClick={() => void remove()}>
                Delete taxonomy
              </button>
            </div>
          </Show>
        </header>
        <div class="form-grid">
          <Show when={isNew()}>
            <label for="taxonomy-name">Name</label>
            <input id="taxonomy-name" value={name()} placeholder="status" onInput={(e) => setName(e.currentTarget.value)} />
          </Show>
          <label for="taxonomy-label">Label</label>
          <input id="taxonomy-label" value={model.label} onInput={(e) => edit((m) => (m.label = e.currentTarget.value))} />
          <label for="taxonomy-description">Description</label>
          <input
            id="taxonomy-description"
            value={model.description}
            onInput={(e) => edit((m) => (m.description = e.currentTarget.value))}
          />
          <span />
          <label title="A term typed on a note that this taxonomy lacks is added to it when the note is saved.">
            <input
              type="checkbox"
              checked={model.allow_new}
              onChange={(e) => edit((m) => (m.allow_new = e.currentTarget.checked))}
            />{" "}
            Let fields add new terms
          </label>
        </div>
        <h2>Terms</h2>
        <Terms terms={model.terms} prefix="" />
        <button type="button" onClick={() => edit((m) => void m.terms.push(newTerm("New term")))}>
          Add term
        </button>
        <div class="save-bar">
          <Show when={issues().length > 0}>
            <ul class="error" role="alert" aria-label="Problems">
              <For each={issues()}>{(p) => <li>{p}</li>}</For>
            </ul>
          </Show>
          <Show when={error()}>
            <p class="error" role="alert">
              {error()}
            </p>
          </Show>
          <UsingNotes notes={using()} label="Notes using it" />
          <Show when={status()}>
            <p role="status">{status()}</p>
          </Show>
          <button type="button" class="primary" disabled={busy() || issues().length > 0} onClick={() => void save()}>
            {isNew() ? "Create taxonomy" : "Save taxonomy"}
          </button>
        </div>
      </Show>
    </section>
  );
}
