import { test } from "node:test";
import assert from "node:assert/strict";
import { contactDiagnostic } from "./contact-diagnostic.mjs";

test("prompt contact returns without snapshot or second wait", async () => {
  const person = { id: "bob" };
  const find = () => person;
  assert.equal(await contactDiagnostic(find, async (_, fn, options) => {
    assert.equal(options.timeout, 30_000);
    return fn();
  }, () => assert.fail("unexpected snapshot")), person);
});

test("slow contact snapshots before an unnudged second wait", async () => {
  const order = [];
  const person = { id: "bob" };
  assert.equal(await contactDiagnostic(() => person, async (_, find, options) => {
    order.push(options.timeout);
    if (options.timeout === 30_000) throw new Error("timeout");
    return find();
  }, () => order.push("snapshot")), person);
  assert.deepEqual(order, [30_000, "snapshot", 90_000]);
});

test("second timeout remains a failure", async () => {
  let snapshots = 0;
  await assert.rejects(contactDiagnostic(() => null, async () => {
    throw new Error("timeout");
  }, () => snapshots++), /timeout/);
  assert.equal(snapshots, 1);
});
