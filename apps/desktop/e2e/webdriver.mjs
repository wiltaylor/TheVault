// A small W3C WebDriver client over fetch, enough to drive the desktop app through tauri-driver.

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

/** Polls `check` until it returns a truthy value, and returns it; fails after `timeout` ms. */
export async function waitFor(what, check, timeout = 30_000) {
  const end = Date.now() + timeout;
  let last;
  for (;;) {
    try {
      const value = await check();
      if (value) return value;
      last = value;
    } catch (e) {
      last = e;
    }
    if (Date.now() > end) throw new Error(`timed out waiting for ${what}; last saw: ${last instanceof Error ? last.message : JSON.stringify(last)}`);
    await sleep(200);
  }
}

export class Session {
  constructor(base, id) {
    this.base = base;
    this.id = id;
  }

  static async start(base, capabilities) {
    const res = await fetch(`${base}/session`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ capabilities: { alwaysMatch: capabilities } }),
    });
    const body = await res.json();
    if (!res.ok) throw new Error(`new session failed: ${JSON.stringify(body)}`);
    return new Session(base, body.value.sessionId);
  }

  async cmd(method, path, body) {
    const res = await fetch(`${this.base}/session/${this.id}${path}`, {
      method,
      headers: { "content-type": "application/json" },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
    const answer = await res.json();
    if (!res.ok) throw new Error(`${method} ${path}: ${JSON.stringify(answer.value)}`);
    return answer.value;
  }

  url() {
    return this.cmd("GET", "/url");
  }

  navigate(url) {
    return this.cmd("POST", "/url", { url });
  }

  /** The element `css` selects, as the reference WebDriver returns. */
  find(css) {
    return this.cmd("POST", "/element", { using: "css selector", value: css });
  }

  click(element) {
    return this.cmd("POST", `/element/${Object.values(element)[0]}/click`, {});
  }

  type(element, text) {
    return this.cmd("POST", `/element/${Object.values(element)[0]}/value`, { text });
  }

  /** Runs `script` in the page and returns what it returns. */
  run(script, ...args) {
    return this.cmd("POST", "/execute/sync", { script, args });
  }

  /** The text of the element `css` selects, or null when there is none. */
  text(css) {
    return this.run("const el = document.querySelector(arguments[0]); return el ? el.textContent : null;", css);
  }

  quit() {
    return this.cmd("DELETE", "");
  }
}
