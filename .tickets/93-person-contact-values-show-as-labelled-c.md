---
id: '93'
title: Person contact values show as labelled chips you can copy and edit, and image
  fields are a tile with a placeholder that picks or uploads an image
blocked_by: []
status: done
verified: verified
---

# Person contact values show as labelled chips you can copy and edit, and image fields are a tile with a placeholder that picks or uploads an image

The user wants a person's phones, emails and postal addresses to read as one chip per value with its label beside it ("mobile  +61400000020"), not the value chips with a separate label chip list below, and to edit or copy a value from its chip. They also want the photo to be an image tile, not a dropdown of asset filenames: a placeholder when empty, and clicking it lets you pick an existing image asset or upload one.

Today (from a code map): a list-of-string slot renders through NoteView `Slot` (web/src/pages/NoteView.tsx:1132) -> `FieldInput` (web/src/components/FieldInput.tsx:597) -> `Picker`/`ChipPicker` (web/src/components/ChipPicker.tsx:172-191) with add and remove only; `add` skips duplicates and `remove` filters by value (ChipPicker.tsx:82,90). Nothing pairs `phones[i]` with `phone_labels[i]`. The asset field is a picker of filenames (FieldInput.tsx:459-520). `copyText` exists in web/src/blocks/CodeBlock.tsx:257 (Clipboard API with execCommand fallback over plain http). The person type is crates/writer/defaults/person.wcl, copies in crates/cli/starter/types/person.wcl and crates/bench/defs/types/person.wcl.

Build:

1. Schema, a `labels` attribute on a `list` field whose items are `string`: `labels = "<field>"` names another field of the same type that is a list of string, holding one label per value, index-aligned. Parse and check it in crates/schema/src/types.rs (attribute list ~:418-445; the named field must exist, be a list of string, not be the field itself, and not be named by two fields), carry it in FieldDef and the types API JSON (crates/api/src/types.rs field_json and crates/api/src/inspect.rs), the web FieldDef (web/src/api.ts:31-55) and the type editor model (web/src/defs/typeModel.ts). A field named by `labels` is not shown on its own: leave it out of the properties sidebar and do not render a slot for it, even if a template names it. Document the attribute in docs/vault-format.md.

2. Schema, an `accept` attribute on an `asset` field: `"image"` or `"any"` (default `"any"`). With `"image"`, a value must be an asset whose media type is image/*. Same plumbing and docs as 1.

3. Web, labelled list chips, wherever a list field with `labels` is edited (FieldInput and the template control path in web/src/controls/ControlView.tsx:571-591) and shown read-only (Display / formatValue in web/src/components/Render.tsx, cards, tables):
   - One chip per value on one line: the label as a small muted prefix, then the value. A value with no label shows no prefix.
   - Each chip has Copy (copies the value, shows "Copied" briefly, uses copyText), Edit and Remove. Clicking the chip text opens Edit.
   - Edit and Add open one small popover with a label combobox (suggestions: mobile, home, work, main, other, plus labels already used in this vault for the field; free text allowed) and a value input; Enter saves, Esc cancels.
   - Every operation works by index and writes both lists in one save (`setField` for both fields in one write), so duplicate values are allowed and labels never drift from their values. An empty list removes both fields, as an empty list does today.
   - Read-only places show "label value" pairs, e.g. "mobile +61400000020, home +61400000021".

4. Web, image asset fields (`accept = "image"`):
   - A square image tile showing the asset as a thumbnail; when empty, a placeholder (an image icon and "Add photo" for a field labelled Photo, otherwise "Add image").
   - Clicking the tile opens a dialog with Upload (file picker and drag and drop, images only) and a searchable grid of existing image assets; choosing one sets the field. A set tile also offers Remove and Open full size.
   - Cards and tables show the thumbnail.
   - Keep the existing picker for `accept = "any"` asset fields.

5. Person type: `emails labels = "email_labels"`, `phones labels = "phone_labels"`, `addresses labels = "address_labels"`, `photo accept = "image"`; drop the label fields' slots from the default template; put `photo` first in the default and card templates. Same in the starter and bench copies.

6. Existing vaults: register upgrade step `0002_person_labels` in crates/writer/src/shipped.rs `UPGRADE_STEPS` that, when the vault's person type has `phones`, `emails`, `addresses` and their label lists but no `labels` attributes, adds the three `labels` attributes and `accept = "image"` on `photo` (only when every existing photo value is an image asset), and removes slots for the three label fields from the default template, as a planned type change that edits no note. A person type the user has changed in other ways is still upgraded on these points only.

Acceptance criteria:
- Schema tests: `labels` accepted on a list of string naming a list of string; refused naming a missing field, a non-list, itself, or a field another field already names; `accept = "image"` refuses a non-image asset value.
- Web component tests: a labelled list renders one chip per value with its label prefix; Copy calls copyText with the value; Edit changes the label and the value of that index only and saves both lists in one write; Add appends to both; Remove at an index removes the same index from both even with duplicate values.
- Web component test: an empty image field shows the placeholder; picking an existing image asset sets it; upload sets it; Remove clears it.
- An upgrade test: a vault with the 0.1.6 person type gains the three `labels` attributes, `accept = "image"` and the slot changes, no note changes, a second start does nothing.
- A Playwright check (web/e2e) on a person note: chips show "mobile" beside the number, Copy and Edit work, and the photo tile opens the picker. Save a screenshot under .scratch/shots/ for the user.
- `./dev just check` passes.

## Worklog

### 2026-10-17T22:39:05Z — /work

Claimed.

### 2026-10-17T23:03:30Z — wil

Verified set to verified.

### 2026-10-17T23:03:30Z — wil

Status set to done.

### 2026-10-17T23:03:30Z — wil

Done on ticket/93 at 47d71eb. Schema tests (crates/schema/tests/validation.rs: labels_*, an_image_asset_field_*), web tests (LabelledList.test.tsx, ImageField.test.tsx, defs.test.ts), upgrade tests (crates/writer/tests/shipped_defaults.rs: a_0_1_6_vault_gains_labels_*, a_photo_that_is_not_an_image_*), Playwright web/e2e/labelled-chips.spec.ts; ./dev just check and the full e2e suite (307) pass. Step 0001 now strips labels from the emails block it copies so it stays valid; 0002 adds missing label fields itself. Seeding now places a new slot before the next shipped slot when none precedes it (photo before title). Left alone: crates/bench/defs/types/person.wcl (old minimal person with no phones/photo); no type-editor UI for labels/accept (model only); multiple image-only asset fields keep the chip picker. Filed 100 on older devices and FORMAT_VERSION.
