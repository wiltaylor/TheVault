// The chat's Markdown: GitHub-flavoured Markdown through marked (tables, nested and task lists,
// strikethrough, rules, quotes, fenced code), then DOMPurify, so an answer cannot inject script,
// styles or event handlers. Links to an app path (/notes/<id>, /agents/...) stay plain in-app
// links, which the workspace opens in the active pane; http, https and mailto links open in a new
// browser tab; any other link shows as its text. Only the vault's own images (/api/assets/...)
// load: an answer cannot make the browser fetch another site's image, so a remote image shows
// as its alt text.

import DOMPurify from "dompurify";
import { Marked } from "marked";
import { createMemo } from "solid-js";

const md = new Marked({ gfm: true, breaks: true });

/** Only these link targets stay links; anything else shows as its text. */
const SAFE_HREF = /^(https?:|mailto:|\/(?!\/))/i;
/** The only images an answer may show: the vault's own assets. */
const SAFE_IMG = /^\/api\/assets\//;

// An instance of its own, so these hooks touch no other use of DOMPurify.
const purify = DOMPurify(window);

purify.addHook("afterSanitizeAttributes", (node) => {
  if (node.tagName === "A") {
    const href = node.getAttribute("href") ?? "";
    if (!SAFE_HREF.test(href)) {
      node.replaceWith(...Array.from(node.childNodes));
      return;
    }
    if (!href.startsWith("/")) {
      node.setAttribute("target", "_blank");
      node.setAttribute("rel", "noopener noreferrer external");
    }
  } else if (node.tagName === "IMG") {
    const src = node.getAttribute("src") ?? "";
    if (!SAFE_IMG.test(src)) node.replaceWith(document.createTextNode(node.getAttribute("alt") ?? ""));
    else node.setAttribute("loading", "lazy");
  } else if (node.tagName === "INPUT") {
    // A task list's box shows the state; the answer is not a form.
    node.setAttribute("disabled", "");
  }
});

const OPTIONS = {
  FORBID_TAGS: ["style", "form", "button", "textarea", "select", "iframe", "object", "embed"],
  FORBID_ATTR: ["style", "id", "name"],
  RETURN_DOM_FRAGMENT: true as const,
};

/** `text` as sanitised HTML. An unclosed code fence runs to the end, as it does while streaming. */
export function renderMarkdown(text: string): string {
  const html = md.parse(text, { async: false });
  const fragment = purify.sanitize(html, OPTIONS);
  // A wide table scrolls in its own box rather than widening the panel.
  for (const table of Array.from(fragment.querySelectorAll("table"))) {
    const box = document.createElement("div");
    box.className = "ai-md-table";
    table.replaceWith(box);
    box.append(table);
  }
  const out = document.createElement("div");
  out.append(fragment);
  return out.innerHTML;
}

/** `text` as Markdown. */
export function Markdown(props: { text: string }) {
  const html = createMemo(() => renderMarkdown(props.text));
  return <div class="ai-md" innerHTML={html()} />;
}
