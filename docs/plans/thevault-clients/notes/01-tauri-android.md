---
id: "01"
kind: research
---

# Tauri 2 on Android: what TheVault's Android app needs

## Question

Can a Tauri 2 mobile app that reuses the SolidJS front end in its WebView do all of these: (a) act as a share target, (b) read the call log and react to call end, (c) read and receive SMS/MMS (and what about RCS), (d) read, write and observe contacts, (e) show and cancel notifications from server events while the app is closed, (f) run background work, and (g) run Rust (such as a script interpreter) in-process? For each item, is there a plugin, or do we write our own Kotlin plugin?

## Answer

| Item | Plugin available | Verdict |
|---|---|---|
| (a) Share target | `tauri-plugin-mobile-sharetarget` 2.0.0 (IT-ess). It queues intents in Rust, and the front end pulls them with `popIntentQueue()`, so a cold-start share is not lost. `tauri-plugin-sharetarget` 0.1.6 (Nov 2024) is event-based, does not handle SEND_MULTIPLE, and is effectively unmaintained. | Use mobile-sharetarget or write our own. We need SEND_MULTIPLE and reading `content://` streams, so our own small plugin is likely. |
| (b) Call log and call end | None found. | Custom Kotlin plugin. Query `CallLog.Calls` (READ_CALL_LOG). Detect call end with a manifest receiver for `ACTION_PHONE_STATE_CHANGED` (READ_PHONE_STATE), which is exempt from implicit-broadcast limits: on the transition to IDLE, re-query the call log. |
| (c) SMS/MMS | None found. | Custom plugin. Read `Telephony.Sms`/`Mms` (READ_SMS). `SMS_RECEIVED` and `WAP_PUSH_RECEIVED` are exempt broadcasts, so a manifest receiver works. A non-default app is not the one that downloads MMS, so the plugin should observe `content://mms-sms/` rather than parse the WAP push. |
| (c) RCS | None possible. | No public API or provider. RCS content never reaches third-party apps. The only workaround is a `NotificationListenerService` that scrapes Google Messages notifications. It is lossy: there is no notification while the conversation is open, and saved contacts show a name, not a number. |
| (d) Contacts | None found. | Custom plugin over `ContactsContract` (READ/WRITE_CONTACTS). Use a `ContentObserver` while the process is alive. When it is not, use WorkManager `Constraints.addContentUriTrigger` on `ContactsContract.Contacts.CONTENT_URI`. Delta sync uses `CONTACT_LAST_UPDATED_TIMESTAMP` and `DeletedContacts`. |
| (e) Notifications | Official `tauri-plugin-notification` v2 (plugins-workspace) is local only. On Android it supports channels, actions, `cancel` and `removeActive`. For push: `tauri-plugin-notifications` (Choochmeque, 0.5) uses FCM on Android (UnifiedPush only on Linux), `tauri-plugin-mobile-push` (yanqianglu, npm Apr 2026) is FCM-only, and `tauri-plugin-push-notifications` is FCM, async. | Delivery while closed needs FCM or UnifiedPush; none of the Tauri plugins offer UnifiedPush on Android. For a self-hosted, Google-free path, write our own plugin with `org.unifiedpush.android:connector` (3.3.3, which needs Kotlin 2.3 and pulls in Tink). In it, post or cancel a `NotificationCompat` notification with a stable id, all in Kotlin, without starting the WebView. |
| (f) Background work | `tauri-plugin-background-service` 1.0.x (dardourimohamed): a foreground service plus a Tokio task. v1.0.1 has an Android bug: it fails without a "headless core" native lib (fix in PR #53). | Possible, with caveats (see below). WorkManager has no Tauri plugin; call it from our own Kotlin plugin. |
| (g) Rust in-process | Core Tauri. | Yes. The app is a Rust `cdylib` loaded by the Activity. JS calls Rust commands. Rust calls Kotlin via `register_android_plugin` and `run_mobile_plugin`/`run_mobile_plugin_async`. Kotlin pushes to Rust or JS only through `trigger()` events or a `Channel` passed in an earlier invoke. For an unprompted call from Kotlin into Rust, use plain JNI (`System.loadLibrary` plus `external fun`). |

How to write a custom plugin: run `tauri plugin new <name> --android` (or `tauri plugin android init` in an existing plugin). This gives a Kotlin class `@TauriPlugin(permissions = [Permission(strings = [...], alias = "...")]) class X(activity: Activity): Plugin(activity)` with `@Command fun foo(invoke: Invoke)`. Arguments are `@InvokeArg` classes read with `invoke.parseArgs`. Runtime prompts come from the built-in `checkPermissions`/`requestPermissions` commands, keyed by alias. The plugin overrides `load(webView)` and `onNewIntent(intent)`, and emits with `trigger("event", JSObject)`, which JS receives through `addPluginListener`. Manifest entries (permissions, `<receiver>`, `<service>`, intent filters) go in the plugin's own `android/src/main/AndroidManifest.xml` and are merged by Gradle. The share intent filters go in `src-tauri/gen/android/app/src/main/AndroidManifest.xml`.

## Why

- Tauri mobile plugin API: `@TauriPlugin`, `@Command`, permission aliases, `load`, `onNewIntent`, `trigger`, and JNI for calling Rust. https://v2.tauri.app/develop/plugins/develop-mobile/
- A Tauri maintainer says Kotlin cannot call Rust "unprompted"; the workarounds are listeners or `Channel`. https://github.com/tauri-apps/tauri/discussions/10695 . The geolocation plugin shows the Channel pattern: https://github.com/tauri-apps/plugins-workspace/blob/v2/plugins/geolocation/src/mobile.rs
- Rust handlers run before native dispatch, which is a pitfall. https://github.com/meri-leeworthy/tauri-plugin-mobile-push/pull/1
- Minimum SDK: Tauri supports Android 7.0 (API 24), configurable with `bundle.android.minSdkVersion`. https://v2.tauri.app/distribute/google-play/
- Share target plugins: https://lib.rs/crates/tauri-plugin-sharetarget , https://github.com/IT-ess/tauri-plugin-mobile-sharetarget , and the content-URI streaming discussion: https://github.com/tauri-apps/tauri/discussions/11548
- Notification plugins: https://v2.tauri.app/plugin/notification/ , https://github.com/Choochmeque/tauri-plugin-notifications , https://github.com/yanqianglu/tauri-plugin-mobile-push , https://docs.rs/tauri-plugin-push-notifications/latest/tauri_plugin_push_notifications/
- UnifiedPush connector: https://central.sonatype.com/artifact/org.unifiedpush.android/connector
- Exempt implicit broadcasts (`ACTION_PHONE_STATE_CHANGED`, `SMS_RECEIVED_ACTION`, `WAP_PUSH_RECEIVED_ACTION`): https://developer.android.com/develop/background-work/background-tasks/broadcasts/broadcast-exceptions
- RCS has no third-party access, and notification scraping fails in known ways: https://github.com/gideontek/phonetrack/pull/14 , https://www.xda-developers.com/google-messages-rcs-api-third-party-apps/
- Background service plugin and its v1.0.1 bug: https://crates.io/crates/tauri-plugin-background-service , https://github.com/dardourimohamed/tauri-background-service/pull/53
- When the last Activity is destroyed, Tauri calls `exit(0)` unless `RunEvent::ExitRequested` → `api.prevent_exit()` is used. Relaunching the UI in a process kept alive by a foreground service gives a blank WebView (open): https://github.com/tauri-apps/tauri/issues/15671 . An older related leak: https://github.com/tauri-apps/tauri/issues/11609
- Play policy: SMS and Call Log permissions are allowed only for default handlers or listed exceptions, and the policy covers Play distribution only. https://support.google.com/googleplay/android-developer/answer/10208820
- Android 13 and 15 "restricted settings": for sideloaded apps, the SMS permission and notification-listener access are blocked until the user enables them manually. https://www.androidauthority.com/android-15-restricted-settings-sideloading-3481098/
- Developer verification is enforced from 30 Sep 2026 in BR, ID, SG and TH, with global rollout in 2027. ADB installs are exempt, and an "advanced flow" exists. https://developer.android.com/blog/posts/android-developer-verification-rolling-out-to-all-developers-on-play-console-and-android-developer-console , https://www.androidauthority.com/android-sideloading-developer-verification-first-wave-rollout-3717921/

## Consequences

- Most of the Android-specific work is one custom Kotlin plugin (or a few), for example `tauri-plugin-vault-android` covering share, telephony, contacts, push and work. Only notifications have a usable official plugin.
- **Background code must not depend on the WebView or the Tauri runtime.** Broadcast receivers, `FirebaseMessagingService`/UnifiedPush receivers and WorkManager workers run in a process where the Activity, and so the Tauri Rust runtime, may never have started. Two options:
  1. Do background work in Kotlin: talk to the server over HTTP and post notifications natively.
  2. Load the app's Rust lib through JNI to get a "headless core", the pattern used by mobile-sharetarget and background-service.

  Option 2 is the only way to run the embedded script interpreter in the background. It needs a JNI surface separate from Tauri commands. Open question: which path?
- Captured events (shares, calls, SMS, contact changes) that arrive while the WebView is down must be queued durably (a file or SQLite in app storage) and drained later, either to the server or on the front end's next load. A pull-based design is better than events for cold start.
- Distribution decides permissions. Play would reject READ_SMS and READ_CALL_LOG unless the app is the default handler. So the app is sideloaded or comes from F-Droid or Obtainium: it is subject to restricted settings (the user must allow SMS by hand) and, from 2027, to developer verification. Open questions: register as a verified developer, or document the ADB/advanced flow?
- RCS capture is out of scope beyond an optional, explicitly lossy notification-listener mode. Open question: ship that mode at all?
- Push needs a decision: FCM (needs a Firebase project and `google-services.json`, and the self-hosted server needs service-account credentials) or UnifiedPush (user installs a distributor such as ntfy; fits self-hosting), or both. Cancel-on-clear-elsewhere means the server must send "dismiss id X" pushes as well as "show" pushes, and the app must use stable notification ids derived from server event ids.
- A long-running foreground service hits Tauri issue #15671 (blank UI on relaunch). Prefer WorkManager and push-triggered short work over a persistent service unless that issue is fixed. Android 15 limits `dataSync` foreground services to 6 h per day.
- Shared files arrive as `content://` URIs. The plugin must copy or stream them (in Kotlin or Rust through a file descriptor) to the server's upload API. The WebView cannot read them directly.
- minSdk: Tauri's floor is 24. Open question: raise it (for example to 26 or 29) to simplify notification channels, the WorkManager content trigger (API 24+) and storage handling?
