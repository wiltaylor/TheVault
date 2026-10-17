// The code block on CodeMirror 6 (D29). The source is kept verbatim: CodeMirror holds it as typed,
// tabs and trailing spaces included, and Tab inserts a tab character rather than leaving the
// editor. A shell session is two code blocks: a `bash` one for the command and a `stdout` one for
// what it printed, which shows as plain program output.

import { indentLess, insertTab } from "@codemirror/commands";
import { HighlightStyle, LanguageDescription, type LanguageSupport, syntaxHighlighting } from "@codemirror/language";
import { languages } from "@codemirror/language-data";
import { Compartment, EditorState, Prec } from "@codemirror/state";
import { EditorView, keymap } from "@codemirror/view";
import { classHighlighter, tags as t } from "@lezer/highlight";
import { basicSetup, minimalSetup } from "codemirror";
import { For, Show, createEffect, createSignal, on, onCleanup, onMount } from "solid-js";
import type { Block } from "../api";
import { text, withItem, withText } from "./fields";
import { copyText } from "../clipboard";

/** The `language` of a code block that holds program output rather than code. */
export const OUTPUT = "stdout";

/** Whether a block's `language` item marks program output: no highlighting, no line numbers. */
export function isOutput(language: string): boolean {
  return language.trim().toLowerCase() === OUTPUT;
}

/** The CodeMirror language a block's `language` item names, by name or alias, any case. */
export function findLanguage(name: string): LanguageDescription | null {
  if (!name.trim() || isOutput(name)) return null;
  return LanguageDescription.matchLanguageName(languages, name.trim(), false);
}

/**
 * The value a block stores for a language: its CodeMirror name in lower case, except the shell,
 * which is stored as `bash`, the name a shell command block is inserted with.
 */
export function languageValue(lang: LanguageDescription): string {
  return lang.name === "Shell" ? "bash" : lang.name.toLowerCase();
}

/** A language as the picker and the header name it: the shell as Bash. */
export function languageName(lang: LanguageDescription): string {
  return lang.name === "Shell" ? "Bash" : lang.name;
}

const LANGUAGES = [...languages].sort((a, b) => languageName(a).localeCompare(languageName(b)));

async function loadLanguage(name: string): Promise<LanguageSupport | []> {
  const lang = findLanguage(name);
  if (!lang) return [];
  try {
    return await lang.load();
  } catch {
    return [];
  }
}

/**
 * CodeMirror drawn from the app's CSS variables, so it follows the light and dark themes: the
 * gutters, active line, selection and cursor, and (below) the syntax colours.
 */
const appTheme = EditorView.theme({
  "&": { color: "var(--ink)", backgroundColor: "var(--panel)" },
  ".cm-content": { caretColor: "var(--ink)" },
  ".cm-cursor, .cm-dropCursor": { borderLeftColor: "var(--ink)" },
  ".cm-gutters": { backgroundColor: "var(--bg)", color: "var(--muted)", borderRight: "1px solid var(--line)" },
  ".cm-activeLine": { backgroundColor: "var(--code-active-line)" },
  ".cm-activeLineGutter": { backgroundColor: "var(--hover)", color: "var(--ink)" },
  "&.cm-focused > .cm-scroller > .cm-selectionLayer .cm-selectionBackground, .cm-selectionBackground, ::selection": {
    backgroundColor: "var(--code-selection)",
  },
  ".cm-selectionMatch": { backgroundColor: "var(--accent-soft)" },
  "&.cm-focused .cm-matchingBracket": { backgroundColor: "var(--accent-soft)", outline: "1px solid var(--line)" },
  ".cm-foldPlaceholder": { backgroundColor: "var(--hover)", color: "var(--muted)", border: "1px solid var(--line)" },
  ".cm-tooltip": { backgroundColor: "var(--panel)", color: "var(--ink)", border: "1px solid var(--line)" },
  ".cm-tooltip-autocomplete ul li[aria-selected]": { backgroundColor: "var(--accent-soft)", color: "var(--ink)" },
  ".cm-panels": { backgroundColor: "var(--bg)", color: "var(--ink)" },
  ".cm-searchMatch": { backgroundColor: "var(--highlight)" },
});

/** Syntax colours from CSS variables, readable on both themes' backgrounds. */
const appHighlight = HighlightStyle.define([
  { tag: [t.keyword, t.controlKeyword, t.moduleKeyword, t.operatorKeyword, t.modifier], color: "var(--code-keyword)" },
  { tag: [t.string, t.special(t.string), t.regexp, t.character], color: "var(--code-string)" },
  { tag: [t.number, t.bool, t.null, t.atom], color: "var(--code-number)" },
  { tag: [t.comment, t.lineComment, t.blockComment, t.docComment], color: "var(--code-comment)", fontStyle: "italic" },
  { tag: [t.function(t.variableName), t.function(t.propertyName), t.macroName], color: "var(--code-function)" },
  { tag: [t.typeName, t.className, t.namespace, t.tagName], color: "var(--code-type)" },
  { tag: [t.propertyName, t.attributeName, t.labelName], color: "var(--code-property)" },
  { tag: [t.definition(t.variableName), t.variableName], color: "var(--ink)" },
  { tag: [t.operator, t.punctuation, t.bracket], color: "var(--muted)" },
  { tag: [t.meta, t.processingInstruction], color: "var(--code-comment)" },
  { tag: t.heading, color: "var(--code-keyword)", fontWeight: "bold" },
  { tag: t.emphasis, fontStyle: "italic" },
  { tag: t.strong, fontWeight: "bold" },
  { tag: t.link, color: "var(--accent)", textDecoration: "underline" },
  { tag: t.invalid, color: "var(--danger)" },
]);

/**
 * A CodeMirror view of `source`, editable when `onChange` is given. As `output`, it is program
 * output: no language, no gutters, and long lines wrap.
 */
export function CodeMirror(props: {
  source: string;
  language: string;
  output?: boolean;
  onChange?: (source: string) => void;
}) {
  let host!: HTMLDivElement;
  let view: EditorView | undefined;
  const language = new Compartment();

  onMount(() => {
    const editable = props.onChange !== undefined;
    const output = props.output === true;
    const common = [
      language.of([]),
      appTheme,
      syntaxHighlighting(appHighlight),
      // tok-* classes as well, so a highlighted token can be found by what it is.
      syntaxHighlighting(classHighlighter),
      EditorView.contentAttributes.of({ "aria-label": output ? "Output" : "Code" }),
      ...(output ? [EditorView.lineWrapping] : []),
    ];
    const extensions = editable
      ? [
          Prec.high(keymap.of([{ key: "Tab", run: insertTab, shift: indentLess }])),
          output ? minimalSetup : basicSetup,
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
          const support = output ? [] : await loadLanguage(name);
          if (view && name === props.language) view.dispatch({ effects: language.reconfigure(support) });
        },
      ),
    );
  });
  onCleanup(() => view?.destroy());

  return <div ref={host} class="code-mirror" classList={{ "code-output": props.output === true }} />;
}

/**
 * A code block. Away from the cursor it reads like any code block: the file name and language in
 * a slim header over the code. While the cursor is in it, the language selector and file name
 * field show above the code, with line numbers.
 */
export function CodeBlock(props: { block: Block; onChange?: (block: Block) => void }) {
  const [editing, setEditing] = createSignal(false);
  const output = () => isOutput(text(props.block, "language"));
  const known = () => findLanguage(text(props.block, "language"));
  const selected = () => {
    if (output()) return OUTPUT;
    const lang = known();
    return lang ? lang.name : text(props.block, "language");
  };
  const languageLabel = () => {
    if (output()) return "Output";
    const lang = known();
    return lang ? languageName(lang) : text(props.block, "language");
  };
  const set = (item: string, value: string) => props.onChange?.(withText(props.block, item, value));
  const mirror = (asOutput: boolean) => (
    <CodeMirror
      source={text(props.block, "source")}
      language={text(props.block, "language")}
      output={asOutput}
      onChange={props.onChange ? (source) => props.onChange?.(withItem(props.block, "source", source)) : undefined}
    />
  );

  return (
    <figure
      class="block-code"
      classList={{ editing: editing() && !!props.onChange, "block-output": output() }}
      onFocusIn={() => setEditing(true)}
      onFocusOut={(e) => {
        const to = e.relatedTarget;
        if (!(to instanceof Node && e.currentTarget.contains(to))) setEditing(false);
      }}
    >
      <Show
        when={props.onChange && editing()}
        fallback={
          <figcaption class="code-header">
            <span class="code-filename">{text(props.block, "caption")}</span>
            <span class="code-language">{languageLabel()}</span>
            <CopyButton source={() => text(props.block, "source")} />
          </figcaption>
        }
      >
        <div class="block-fields code-fields">
          <select
            aria-label="Language"
            onChange={(e) => {
              const value = e.currentTarget.value;
              const lang = LANGUAGES.find((l) => l.name === value);
              set("language", value === OUTPUT ? OUTPUT : lang ? languageValue(lang) : "");
            }}
          >
            <option value="" selected={selected() === ""}>
              Plain text
            </option>
            <option value={OUTPUT} selected={selected() === OUTPUT}>
              Output
            </option>
            {/* A language CodeMirror does not know stays as written, unhighlighted. */}
            <Show when={!known() && !output() && text(props.block, "language")}>
              <option value={text(props.block, "language")} selected>
                {text(props.block, "language")}
              </option>
            </Show>
            <For each={LANGUAGES}>
              {(l) => (
                <option value={l.name} selected={selected() === l.name}>
                  {languageName(l)}
                </option>
              )}
            </For>
          </select>
          <input
            aria-label="Caption"
            placeholder="File name"
            value={text(props.block, "caption")}
            onInput={(e) => set("caption", e.currentTarget.value)}
          />
        </div>
      </Show>
      {/* Output and code are set up differently, so switching between them remounts the view. */}
      <Show when={output()} fallback={mirror(false)}>
        {mirror(true)}
      </Show>
    </figure>
  );
}

export { copyText } from "../clipboard";

/** Copies the code block's source, then says so for a moment. */
function CopyButton(props: { source: () => string }) {
  const [state, setState] = createSignal<"idle" | "copied" | "failed">("idle");
  let timer: ReturnType<typeof setTimeout> | undefined;
  onCleanup(() => clearTimeout(timer));
  return (
    <button
      type="button"
      class="code-copy"
      aria-label="Copy code"
      title="Copy code"
      onMouseDown={(e) => e.preventDefault()}
      onClick={async () => {
        setState((await copyText(props.source())) ? "copied" : "failed");
        clearTimeout(timer);
        timer = setTimeout(() => setState("idle"), 1500);
      }}
    >
      {state() === "copied" ? "Copied" : state() === "failed" ? "Copy failed" : "Copy"}
    </button>
  );
}
