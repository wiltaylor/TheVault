---
id: "12"
decision: D10
---

# Android distribution and signing

## Question
How is the APK distributed and signed?

## Answer
Own APKs signed with a permanent keystore, published on GitHub Releases; installed by `adb` or tracked with Obtainium. The keystore is backed up offline. F-Droid may come later. No Google Play.

## Why
Play only grants SMS and call-log permissions to default handlers or narrow exceptions (note 02). Single-user app.

## Consequences
Onboarding walks the user through Allow restricted settings for SMS and notification-listener access (Android 13+/15). Register a free Limited Distribution developer account before the 2027 verification rollout.
