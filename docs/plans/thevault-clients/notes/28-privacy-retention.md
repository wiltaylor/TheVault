---
id: "28"
decision: D25
---

# Privacy and retention

## Question
What phone data is kept, for how long, and what is excluded?

## Answer
Everything is kept with no automatic deletion. Settings > Devices holds an exclusion list (contacts, numbers or threads) never captured, and a default editable pattern skips one-time codes and verification texts. Deleting a `message` note deletes it everywhere in the vault but never touches the phone's own SMS store.

## Why
Own server, so keeping everything is the point; skipping verification codes avoids piling up one-time passwords.

## Consequences
The exclusion list and pattern are device settings stored on the server and synced to the device.
