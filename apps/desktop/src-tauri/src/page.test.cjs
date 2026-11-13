const { test } = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const vm = require("node:vm");
const path = require("node:path");

for (const message of [null, "The home feed is not authenticated over plain HTTP."]) {
  test(`startup HTTP notice renders only when supplied: ${Boolean(message)}`, async () => {
    const listeners = [];
    const notices = [];
    const window = {};
    window.top = window;
    const document = {
      readyState: "loading",
      addEventListener(_, fn) { listeners.push(fn); },
      body: { append(box) { notices.push(box); } },
      createElement() {
        return {
          style: {}, children: [], attributes: {},
          setAttribute(k, v) { this.attributes[k] = v; },
          append(...nodes) { this.children.push(...nodes); },
          remove() { this.removed = true; },
        };
      },
    };
    vm.runInNewContext(fs.readFileSync(path.join(__dirname, "page.js"), "utf8"), {
      window, document, location: { protocol: "vault:", pathname: "/" },
      fetch: async (url, options) => {
        assert.equal(url, "/_desktop/http-warning");
        assert.equal(options.method, "POST");
        return { json: async () => ({ message }) };
      },
    });
    await listeners[0]();
    assert.equal(notices.length, message ? 1 : 0);
    if (message) {
      const box = notices[0];
      assert.equal(box.attributes.role, "alert");
      assert.equal(box.children[0].textContent, message);
      box.children[1].onclick();
      assert.equal(box.removed, true);
    }
  });
}
