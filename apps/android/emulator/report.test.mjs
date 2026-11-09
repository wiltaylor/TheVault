import test from "node:test";
import assert from "node:assert/strict";
import { resultsTap } from "./report.mjs";

const titles = { 1: { doing: "Pair" }, 53: { doing: "Future check" } };

test("recorded results keep their verdict and note", () => {
  for (const [status, verdict, skip] of [["pass", "ok", ""], ["fail", "not ok", ""], ["na", "ok", " # SKIP na"], ["partial", "ok", " # SKIP partial"]]) {
    assert.equal(resultsTap({ 1: titles[1] }, { 1: { status, note: "Evidence" } }, { completed: true }),
      `TAP version 13\n1..1\n${verdict} 1 - Check 1: Pair${skip}\n# Evidence\n`);
  }
});

test("absent check in a completed run is skipped as not implemented", () => {
  assert.equal(resultsTap({ 53: titles[53] }, {}, { completed: true }),
    "TAP version 13\n1..1\nok 1 - Check 53: Future check # SKIP not implemented\n# Not implemented.\n");
});

test("absent check after an early stop is a failure", () => {
  assert.equal(resultsTap({ 53: titles[53] }, {}, { completed: false }),
    "TAP version 13\n1..1\nnot ok 1 - Check 53: Future check\n# The run stopped before this check.\n");
});

test("selected checks exclude unreported setup and other titles", () => {
  assert.equal(resultsTap(titles, { 1: { status: "pass", note: "Setup" } }, { only: new Set([53]), completed: true }),
    "TAP version 13\n1..1\nok 1 - Check 53: Future check # SKIP not implemented\n# Not implemented.\n");
});
