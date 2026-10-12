---
id: "17"
decision: D14
---

# Templates

## Question
Q17: How many templates per type?

## Answer
Several named templates per type: a default, plus card and detail variants used by query views. No cross-type fragments.

## Why
Card and list views need a compact rendering. Fragments can come later.

## Consequences
Templates are WYSIWYG-edited in the UI and WCL-edited via the CLI, like all definitions.
