---
id: "44"
decision: D41
---

# Inline markup syntax

## Question
OQ4a: What syntax does inline formatting use inside paragraph strings?

## Answer
A Markdown-like inline syntax: `**bold**`, `*italic*`, `++underline++`, `~~strike~~`, `` `code` ``, `==highlight==`, `{red}coloured text{/}` from a fixed palette, `[label](https://url)`, and note mentions `[[ULID]]` or `[[ULID|label]]`. Backslash escapes a markup character. Headings, lists and other structure are WCL blocks, not markup.

## Why
Defaulted: the user asked the agent to design it. Familiar to agents from Markdown and Obsidian, and small enough to round-trip through the WYSIWYG editor without loss.

## Consequences
A mention without a label renders the target note's current title, so renames need no rewrite.
