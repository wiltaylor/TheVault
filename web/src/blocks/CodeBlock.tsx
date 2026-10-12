// The code block on CodeMirror 6 (D29). The source is kept verbatim: CodeMirror holds it as typed,
// tabs and trailing spaces included, and Tab inserts a tab character rather than leaving the
// editor.

import { indentLess, insertTab } from "@codemirror/commands";
import {
  LanguageDescription,
  type LanguageSupport,
  defaultHighlightStyle,
  syntaxHighlighting,
} from "@codemirror/language";
import { languages } from "@codemirror/language-data";
import { Compartment, EditorState, Prec } from "@codemirror/state";
import { EditorView, keymap } from "@codemirror/view";
import { classHighlighter } from "@lezer/highlight";
import { basicSetup } from "codemirror";
import { For, Show, createEffect, on, onCleanup, onMount } from "solid-js";
import type { Block } from "../api";
import { text, withItem, withText } from "./fields";

/** The CodeMirror language a block's `language` item names, by name or alias, any case. */
export function findLanguage(name: string): LanguageDescription | null {
  return name.trim() ? LanguageDescription.matchLanguageName(languages, name.trim(), false) : null;
}

/** The value a block stores for a language: its CodeMirror name in lower case. */
export function languageValue(lang: LanguageDescription): string {
  return lang.name.toLowerCase();
}

const LANGUAGES = [...languages].sort((a, b) => a.name.localeCompare(b.name));

async function loadLanguage(name: string): Promise<LanguageSupport | []> {
  const lang = findLanguage(name);
  if (!lang) return [];
  try {
    return await lang.load();
  } catch {
    return [];
  }
}

/** A CodeMirror view of `source`, editable when `onChange` is given. */
export function CodeMirror(props: { source: string; language: string; onChange?: (source: string) => void }) {
  let host!: HTMLDivElement;
  let view: EditorView | undefined;
  const language = new Compartment();

  onMount(() => {
    const editable = props.onChange !== undefined;
    const common = [
      language.of([]),
      syntaxHighlighting(defaultHighlightStyle),
      // tok-* classes as well, so a highlighted token can be found by what it is.
      syntaxHighlighting(classHighlighter),
      EditorView.contentAttributes.of({ "aria-label": "Code" }),
    ];
    const extensions = editable
      ? [
          Prec.high(keymap.of([{ key: "Tab", run: insertTab, shift: indentLess }])),
          basicSetup,
          ...common,
          EditorView.updateListener.of((u) => {
            if (u.docChanged) props.onChange?.(u.state.doc.toString());
          }),
        ]
      : [EditorState.readOnly.of(true), EditorView.editable.of(false), ...common];
    view = new EditorView({ parent: host, state: EditorState.create({ doc: props.source, extensions }) });

    createEffect(
      on(
        () => props.source,
        (source) => {
          if (view && source !== view.state.doc.toString()) {
            view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: source } });
          }
        },
        { defer: true },
      ),
    );
    createEffect(
      on(
        () => props.language,
        async (name) => {
          const support = await loadLanguage(name);
          if (view && name === props.language) view.dispatch({ effects: language.reconfigure(support) });
        },
      ),
    );
  });
  onCleanup(() => view?.destroy());

  return <div ref={host} class="code-mirror" />;
}

/** A code block: CodeMirror with a language selector and caption while editing. */
export function CodeBlock(props: { block: Block; onChange?: (block: Block) => void }) {
  const known = () => findLanguage(text(props.block, "language"));
  const selected = () => {
    const lang = known();
    return lang ? lang.name : text(props.block, "language");
  };
  const set = (item: string, value: string) => props.onChange?.(withText(props.block, item, value));

  return (
    <figure class="block-code">
      <Show
        when={props.onChange}
        fallback={
          <Show when={text(props.block, "caption")}>
            <figcaption>{text(props.block, "caption")}</figcaption>
          </Show>
        }
      >
        <div class="block-fields">
          <select
            aria-label="Language"
            onChange={(e) => {
              const lang = LANGUAGES.find((l) => l.name === e.currentTarget.value);
              set("language", lang ? languageValue(lang) : "");
            }}
          >
            <option value="" selected={selected() === ""}>
              Plain text
            </option>
            {/* A language CodeMirror does not know stays as written, unhighlighted. */}
            <Show when={!known() && text(props.block, "language")}>
              <option value={text(props.block, "language")} selected>
                {text(props.block, "language")}
              </option>
            </Show>
            <For each={LANGUAGES}>
              {(l) => (
                <option value={l.name} selected={selected() === l.name}>
                  {l.name}
                </option>
              )}
            </For>
          </select>
          <input
            aria-label="Caption"
            placeholder="Caption or file name"
            value={text(props.block, "caption")}
            onInput={(e) => set("caption", e.currentTarget.value)}
          />
        </div>
      </Show>
      <CodeMirror
        source={text(props.block, "source")}
        language={text(props.block, "language")}
        onChange={props.onChange ? (source) => props.onChange?.(withItem(props.block, "source", source)) : undefined}
      />
    </figure>
  );
}
