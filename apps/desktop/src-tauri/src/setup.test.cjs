const { test } = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const vm = require("node:vm");
const path = require("node:path");

function setup(confirmed, withCode) {
  const elements = new Map();
  const get = (id) => {
    if (!elements.has(id)) elements.set(id, {
      value: "", checked: false, handlers: {},
      addEventListener(event, fn) { this.handlers[event] = fn; },
    });
    return elements.get(id);
  };
  const radios = [{ addEventListener() {} }];
  radios.value = withCode ? "code" : "password";
  get("pair").elements = { with: radios };
  get("server").value = "http://127.0.0.1";
  get("password").value = "fake";
  get("code").value = "fake";
  get("confirm-http").checked = confirmed;
  const requests = [];
  vm.runInNewContext(fs.readFileSync(path.join(__dirname, "setup.js"), "utf8"), {
    document: { getElementById: get },
    fetch: async (url, options) => {
      requests.push({ url, options });
      return { ok: false, status: 400, json: async () => ({ message: "refused" }) };
    },
  });
  return { get, requests };
}

for (const code of [false, true]) {
  for (const confirmed of [false, true]) {
    test(`setup sends explicit confirmation ${confirmed}, code ${code}`, async () => {
      const { get, requests } = setup(confirmed, code);
      await get("pair").handlers.submit({ preventDefault() {} });
      const request = requests.find((r) => r.url === "/_desktop/pair");
      const body = JSON.parse(request.options.body);
      assert.equal(body.confirm_http, confirmed);
      assert.equal(body[code ? "code" : "password"], "fake");
    });
  }
}

test("QR payload cannot confirm HTTP", () => {
  const { get } = setup(false, true);
  get("code").value = JSON.stringify({
    code: "scanned", server_url: "http://127.0.0.1", confirm_http: true,
  });
  get("code").handlers.input();
  assert.equal(get("confirm-http").checked, false);
});

test("confirmation is unchecked in the shipped setup page", () => {
  const html = fs.readFileSync(path.join(__dirname, "setup.html"), "utf8");
  const input = html.match(/<input[^>]+id="confirm-http"[^>]*>/)[0];
  assert.doesNotMatch(input, /\bchecked\b/);
  assert.match(html, /home feed is not authenticated over plain HTTP/);
});
