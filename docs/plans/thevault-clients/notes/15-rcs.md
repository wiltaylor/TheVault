---
id: "15"
decision: D13
---

# RCS

## Question
Ship RCS capture?

## Answer
Yes. The user wants RCS because most messages are RCS now. RCS is captured from Google Messages' notifications through a NotificationListenerService, as event `rcs_received`. It is lossy: messages read while the chat is open on screen produce no notification, senders appear as display names not numbers, attachments are not available, and sent RCS messages are not seen.

## Why
No public RCS API or content provider exists (note 01).

## Consequences
Needs notification-listener access (restricted for sideloaded apps; onboarding covers it). Matching RCS senders to people works by name, not number (Q14). The data model must mark captured RCS as partial (Q12).
