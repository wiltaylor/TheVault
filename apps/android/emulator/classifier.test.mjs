import assert from "node:assert/strict";
import { test } from "node:test";
import { classifierNotes, classifierSynced, assertClassifierView } from "./classifier.mjs";

for (const { type, fields } of classifierNotes) {
  const response = (f) => ({ status: 200, body: JSON.stringify({ note: { fields: f } }) });
  test(`${type}: sync requires both exact classifier values on the phone`, () => {
    assert.equal(classifierSynced(response(fields), fields), true);
    assert.equal(classifierSynced({ status: 404 }, fields), false);
    for (const name of ["scam_probability", "communication_category"]) {
      const missing = { ...fields };
      delete missing[name];
      assert.equal(classifierSynced(response(missing), fields), false);
      assert.equal(classifierSynced(response({ ...fields, [name]: "wrong" }), fields), false);
    }
  });
  test(`${type}: template controls must be visible and contain the values`, async () => {
    const seen = [];
    const page = (wrong) => ({ getByTestId(id) {
      const name = id.replace("field-", "");
      return { locator(selector) {
        seen.push([name, selector]);
        return {
          async waitFor(options) { assert.equal(options.state, "visible"); },
          async inputValue() { return name === wrong ? "wrong" : String(fields[name]); },
        };
      } };
    } });
    await assertClassifierView(page(), fields);
    assert.deepEqual(seen, [["scam_probability", "input"], ["communication_category", "select"]]);
    for (const name of ["scam_probability", "communication_category"]) {
      await assert.rejects(assertClassifierView(page(name), fields), new RegExp(name));
    }
    await assert.rejects(assertClassifierView({ getByTestId() { return { locator() { return { async waitFor() { throw new Error("hidden"); } }; } }; } }, fields), /hidden/);
  });
}
