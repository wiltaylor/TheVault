---
id: "06"
decision: D3
---

# Additional field kinds

## Question
Q6: Add missing field kinds?

## Answer
Add date, datetime, date range, location (lat/lng or image-map coordinates), url and select (inline enum). Add a `computed` flag: UI read-only, written by handlers.

## Why
Calendar, timeline and map blocks need date and location data. Computed fields give handlers a target.

## Consequences
WCL has no date or url type (notes/01-wcl-wscript.md), so these need a representation in WCL or upstream support (Q3). Map tiles source needed (Q28).
