# Android manual checks (S7)

The automated tests run on the JVM (Robolectric) and against a host build of the Rust core. They never install the app on a phone. Run these checks on a real phone with Android 13 or later before each release. Run the restricted settings checks on Android 15 or later as well, where Android blocks SMS access for an app installed from outside Google Play.

## Before you start

1. Build the APK. For a release, build the signed release APK as `docs/android-release.md` describes: `./dev just android-release` writes it to `target/android-release/`. Between releases, the debug APK is enough: `./dev just android-build` writes it to `apps/android/src-tauri/gen/android/app/build/outputs/apk/`.
2. Install it: `adb install -r <apk>`.
3. Run a home server that the phone can reach, and sign in to its web app in a browser.
4. Install a UnifiedPush distributor on the phone, such as ntfy, for the notification checks.
5. Have a second phone, or a friend, who can send you an SMS, an MMS with an image and an RCS message, and call you.

Record the result of each check (pass, or fail with what happened), the phone model and its Android version.

## Onboarding and pairing

| # | Do this | Expect this |
|---|---------|-------------|
| 1 | Open the app for the first time. | The "Connect TheVault to your server" screen opens over the app. |
| 2 | Enter the server address and a wrong password, then tap **Pair**. | "wrong password" appears. The screen stays open. |
| 3 | Enter the correct password and tap **Pair**. | The screen closes. In the web app, Settings > Devices lists the phone with the name you entered and platform `android`. |
| 4 | Revoke the phone in the web app, clear the app's storage, and open the app again. In the web app, choose **Pair a device**. In the app, tap **Scan pairing QR code**, allow the camera and scan the code. | The screen closes. The phone is listed in Settings > Devices again. |
| 5 | Scan any QR code that is not a TheVault pairing code. | "That QR code is not a TheVault pairing code." appears. |
| 6 | Close the app and open it again. | No onboarding screen appears. The app does not ask for the password again. |

## Sync

| # | Do this | Expect this |
|---|---------|-------------|
| 7 | Create a note in the web app, then open the phone app. | The note appears in the app within a few seconds. |
| 8 | Edit a note in the phone app. | The edit appears in the web app within a few seconds. |
| 9 | Turn on airplane mode, then create and edit notes in the app. Turn airplane mode off and wait up to 15 minutes, without opening the app. | The changes reach the web app. Background sync runs every 15 minutes while the phone has a network. |
| 10 | Open a note whose image or PDF the phone has never shown. | The file loads, fetched from the server. |

## Share target

Share from at least three different apps. For example, use Chrome for a link, Photos or Gallery for one image and for several images, and Files for a PDF.

| # | Do this | Expect this |
|---|---------|-------------|
| 11 | Share selected text to **TheVault inbox**. | A "Saved to TheVault inbox" toast appears. The text is a fleeting note in the Inbox. |
| 12 | Share a page from the browser. | A fleeting note titled with the page title, with the link in its body. |
| 13 | Share one image. | A fleeting note that shows the image. |
| 14 | Share several files at once, such as images and a PDF. | One fleeting note with every file attached. |
| 15 | With airplane mode on, share something, then turn airplane mode off. | The note is saved at once. It reaches the web app's Inbox after the next sync. |

## Notifications and UnifiedPush

Install a UnifiedPush distributor first, such as ntfy, and set it up to reach its server. The timed checks need a stopwatch.

| # | Do this | Expect this |
|---|---------|-------------|
| 16 | Open the app after pairing. | If more than one distributor is installed, Android asks which one to use. Afterwards, in the web app, Settings > Devices shows push as on for the phone. |
| 17 | With the app closed and the screen off, create a notification on the server, for example from a script with `vault::notify`. Time it. | An Android notification with the same title and body appears within 10 seconds. |
| 18 | Mark that notification read in the web app's bell. Time it. | The Android notification disappears within 10 seconds. |
| 19 | Create two notifications. Dismiss one in the web app, then clear read notifications. | Each Android notification disappears within 10 seconds of the action that removes it from the server's unread list. |
| 20 | Create a notification, then swipe it away on the phone. | Within a few seconds, the web app's bell shows it as read. With airplane mode on, the read reaches the server once the phone is online again. |
| 21 | Create a notification that opens a note, then tap it on the phone. | The app opens at that note, and the web app's bell shows the notification as read. Repeat with the app already open in the background. |
| 22 | Uninstall the distributor, or onboard on a phone without one. Create a notification on the server and wait up to 15 minutes without opening the app. | The Android notification appears within 15 minutes. The fallback check reads the server's unread list every 15 minutes while the phone has a network. |
| 23 | On a fresh install, refuse the notification permission at the end of onboarding. | Pairing still completes. No Android notifications appear until the permission is granted in the system settings. |

## Allow restricted settings

Run these on Android 15 or later, with the app installed by `adb` or Obtainium rather than Google Play.

| # | Do this | Expect this |
|---|---------|-------------|
| 24 | After pairing, step through the capture setup. On the **Messages** step, tap **Allow**. | Android does not show its permission dialog, or says the setting is restricted. The app shows the three Allow restricted settings steps and an **Open App info** button. |
| 25 | Tap **Open App info**. Tap the three-dot menu at the top right, then **Allow restricted settings**, and confirm with your screen lock. Go back to the app and tap **Try again**. | Android asks for the SMS permissions. Once you allow them, the setup moves to the **Calls** step. |
| 26 | Allow the **Calls**, **Contacts** and **Notifications** steps. Skip **Sending SMS**. | Each step asks for its permissions once and moves on. Skipping **Sending SMS** moves on without asking. |
| 27 | Grant notification access for RCS: in Android's Settings, open **Notifications** > **Device & app notifications** (named **Notification read, reply & control** on some phones), and turn on TheVault. If the switch is greyed out, allow restricted settings from App info as in check 25 first. | TheVault is listed with notification access on. |
| 28 | On the import page, leave the import off and finish. | The app opens. No old messages, calls or contacts arrive in the web app. |

## Messages and calls

Wait a few seconds after each one, then look in the web app.

| # | Do this | Expect this |
|---|---------|-------------|
| 29 | Receive an SMS from a number saved as a contact whose `person` note has that number. | A `message` note with the SMS body, `from` linked to that person, inside a `conversation` note for the thread. |
| 30 | Receive an SMS from a number that no contact or `person` has. | A `message` note with the number in `from_raw`. No new `person` note. |
| 31 | Receive an MMS with an image. | A `message` note in the thread's `conversation` with the image attached and shown. |
| 32 | Receive an RCS message in Google Messages, with the conversation not open on screen. | A `message` note with the sender's name, linked to the person of that name, and `partial` set. |
| 33 | Receive a call and answer it. Then miss a call. Then place a call. | Three `call` notes, with `direction` `in`, `missed` and `out`, linked to the person holding the number. The missed call has `duration` 0. |
| 34 | Restart the phone, then open the app. | No message or call from checks 29 to 33 appears twice. |
| 51 | Delete the `message` note of the SMS from check 29 in the web app. In the app, open Settings > Devices, and under **This device** choose **SMS and MMS messages**, **Last 7 days**, then tap **Import from phone**. | Progress shows per kind, then "Import finished." with how many were found, how many new notes were made and how many were already in the vault. The deleted SMS has a `message` note again, exactly one. No other message appears twice. |
| 52 | Under **This device**, turn off the SMS permission in Android's App info first, then tap **Import from phone** with messages chosen. Then tap **Check for new messages and calls now**. | The import does not start. The page says Android has not given TheVault READ_SMS and offers **Open the permission step**, which opens the capture setup. The check reports how many new items it found. |

## Contacts

| # | Do this | Expect this |
|---|---------|-------------|
| 35 | In the phone's Contacts app, add a contact with a name and a phone number. | A `person` note with that name and number. |
| 36 | Edit that contact's phone number on the phone. | The `person` note's phone changes to match. |
| 37 | In the web app, in Settings > Devices, turn on the phone's contacts-write switch and its `upsert_contact` and `delete_contact` commands. Edit that `person` note's name. | The phone contact's name changes to match within a minute. |
| 38 | Create a `person` note with a phone number in the web app. | A matching contact appears on the phone. |
| 39 | Delete the contact on the phone. | The `person` note stays. It is no longer linked to the phone's contact. |

## Offline capture

| # | Do this | Expect this |
|---|---------|-------------|
| 40 | Turn off Wi-Fi and mobile data, leaving the phone able to receive SMS and calls. Receive an SMS and a call. | No note appears in the web app yet. |
| 41 | Turn Wi-Fi or mobile data back on. | The `message` and `call` notes from check 40 reach the web app after the next sync. |

## Release install

Run these for each release, with the APK attached to its GitHub Release.

| # | Do this | Expect this |
|---|---------|-------------|
| 42 | Install the release APK over the previous release with `adb install -r`, or update it from Obtainium. | The update installs without uninstalling first. The app stays paired, and its notes are still there. |
| 43 | Open **About** in Android's App info for TheVault. | The version matches the release tag. |
| 49 | With the app paired, push working and **Install unknown apps** allowed for TheVault, publish a build with a higher version on the server with `just publish-clients`. Leave the app closed. | Within about a minute, with no tap, the first update over a hand-installed APK too, the phone shows "TheVault updated to <version>". App info shows the new version, installed by TheVault, and the app is still paired with its notes. Publish a second, higher build: it installs the same way. |

## Device commands and contact conflicts

In the web app, open Settings > Devices for the phone. Turn on **Contacts write** and **SMS send**, and enable the `upsert_contact`, `delete_contact`, `send_sms` and `show_notification` commands.

| # | Do this | Expect this |
|---|---------|-------------|
| 44 | On the phone, add a second number to a linked contact. At about the same time, change that person's email address in the web app. | After the next sync, both sides hold both changes: the phone's new number and the web app's new email address. |
| 45 | Delete a linked `person` note in the web app, then edit its contact on the phone. | No `person` note comes back. The contact stays on the phone. |
| 46 | From a server script, queue `send_sms` for the phone with your own number, with the app closed and the screen off. | The text arrives within a few seconds of the push. The web app's command list shows the command `done`, with the result `{ "device_ref": "sms:<id>" }` once Android reported the text sent. |
| 47 | Turn **SMS send** off for the phone and queue `send_sms` again. | No text is sent. The command shows `failed`, and its result says to turn on SMS send. |
| 48 | Uninstall the UnifiedPush distributor and queue `show_notification`. Wait up to 15 minutes without opening the app. | The notification appears within 15 minutes. The fallback check runs pending commands while the phone has a network. |

## Capture health

Run these on a paired phone with capture set up. Open Settings > Devices > **This device** and find **Capture health**.

| # | Do this | Expect this |
|---|---------|-------------|
| 53 | In Android's App info for TheVault, turn off the SMS permission and set the app's battery to **Optimised**. Go back to the app. | Capture health lists **SMS** and **MMS** as **Permission missing**, each with **Open App info**, and **Battery** as **Optimised** with **Stop optimising battery**; on a Samsung phone it also says to keep TheVault out of Sleeping apps. One notification, "TheVault is not capturing everything", appears; tapping it opens Capture health. Closing and opening the app again the same day raises no second one. |
| 54 | Tap **Open App info** and allow SMS again. Tap **Stop optimising battery** and allow it. Go back to the app. | Android's own screens open for each. Back in the app, SMS, MMS and Battery read **Working** and **Not optimised** without reloading. |
| 55 | Repeat check 53, then wait for a sync round, or open the app's sync icon and tap **Sync now**. Open Settings > Devices in the web app in a browser. | The phone's card shows a warning line "The phone reports a capture problem: SMS: permission missing; MMS: permission missing; Battery optimisation is on." with when it was reported. Fix both as in check 54 and sync again: the line goes away. |
| 56 | On a sideloaded install on Android 15 or later, with Allow restricted settings not yet allowed, turn off notification access for TheVault. | Capture health lists **RCS** as **Blocked by restricted settings**, with **Open App info** and **Open notification access**. |
| 57 | With battery optimisation on, leave the phone idle and unplugged, with the app closed, for over two hours. Open the app. | If Android held back the hourly catch-up, Capture health shows **Catch-up of messages, calls and contacts** as **Stalled** with when it last ran, and the notification says so. |

## Coverage

Every check in S7 of the implementation specification, and the checks that cover it:

| S7 check | Checks |
|---|---|
| Share target from three apps | 11 to 15 |
| A live SMS | 29, 30 |
| An MMS with an image | 31 |
| An RCS message | 32 |
| A call | 33 |
| Contact edits in both directions | 35 to 39, 44, 45 |
| Onboarding, including Allow restricted settings | 1 to 6, 24 to 28 |
| A UnifiedPush notification shown and cleared from the web app within seconds | 16 to 20 |
| Offline capture synced later | 9, 15, 40, 41 |
| Import from the phone at any time, recreating a deleted note once | 51, 52 |
