// Read the `wcl check --json` result of a type file on stdin and apply the readers' rule for
// type files (docs/vault-format.md, "Attributes from newer releases"): an attribute a type,
// field, template, tab, slot or fixed template block does not declare is a warning, since a newer
// release may have added it, and the file stays valid. Every other violation, including an
// unknown attribute of a handler, is an error. A list's `item` block and a dict's `values` block
// are not declared in type.wcl (see the note there): the server's schema checks them, so they are
// skipped here.
//
// Prints one line per warning and per error. Exits 0 when there are only warnings.
let input = "";
for await (const chunk of process.stdin) input += chunk;
const result = JSON.parse(input);
const unknown = /^(?:top-level )?field '([^']+)' is not declared by (?:@document )?schema '([^']+)'$/;
const unchecked = /^block kind '(?:item|values)' is not allowed inside '(?:field|key|item|values)'$/;
let errors = 0;
for (const e of result.errors ?? []) {
  if (unchecked.test(e.message)) continue;
  const m = unknown.exec(e.message);
  if (m && m[2] !== "Handler") {
    console.log(`warn  ${e.message}; readers ignore it`);
  } else {
    console.log(`error ${e.message}`);
    errors += 1;
  }
}
process.exit(errors === 0 ? 0 : 1);
