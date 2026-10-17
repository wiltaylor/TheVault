// The clipboard, for every Copy button.

/**
 * Copy `text` to the clipboard. The Clipboard API needs a secure context (HTTPS or localhost), so
 * over plain HTTP, such as a phone on the tailnet, this falls back to a hidden textarea and
 * `execCommand("copy")`.
 */
export async function copyText(text: string): Promise<boolean> {
  if (navigator.clipboard && window.isSecureContext) {
    try {
      await navigator.clipboard.writeText(text);
      return true;
    } catch {
      // Fall through to the older way.
    }
  }
  const area = document.createElement("textarea");
  area.value = text;
  area.setAttribute("readonly", "");
  area.style.position = "fixed";
  area.style.opacity = "0";
  document.body.appendChild(area);
  area.select();
  let ok = false;
  try {
    ok = document.execCommand("copy");
  } catch {
    ok = false;
  }
  area.remove();
  return ok;
}
