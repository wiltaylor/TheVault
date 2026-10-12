---
id: "42"
decision: D39
---

# Benchmark hardware

## Question
OQ2: Which hardware are performance targets measured on?

## Answer
The user's current machine: AMD Ryzen 9 9950X (16 cores, 32 threads), 186 GiB RAM, Samsung 990 EVO Plus 4 TB NVMe, btrfs. The user called it a laptop. The DMI vendor reports an ASUS desktop board.

## Why
User answer to PRD OQ2. Specs read from lscpu, free and lsblk on 2026-10-12.

## Consequences
This is a high-end machine, so the targets say little about smaller servers. The user accepted this.
