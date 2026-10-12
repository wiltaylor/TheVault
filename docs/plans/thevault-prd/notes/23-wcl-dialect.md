---
id: "23"
decision: D20
---

# WCL dialect in vault files

## Question
Q3: Full WCL or a data-only subset?

## Answer
Data-only subset. Notes, types, taxonomies, templates and task definitions contain no expressions and no imports. wscript lives in separate `.ws` files. WCL is used purely as the vault's storage format, not wdoc or any other WCL application.

## Why
Full WCL lets a file read the host disk, and agents write these files. User confirmed WCL is only a data format here.

## Consequences
The server rejects any submitted WCL containing expressions or imports. Cross-file references are validated by TheVault, not WCL (D23).
