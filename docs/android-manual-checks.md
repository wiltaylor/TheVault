# Android manual checks (S7)

The automated tests run on the JVM (Robolectric) and against a host build of the Rust core. They never install the app on a phone. Run these checks on a real phone with Android 13 or later before each release. Later Android tickets add their own sections.

## Before you start

1. Build the debug APK: `./dev just android-build`. The APK is written to `apps/android/src-tauri/gen/android/app/build/outputs/apk/`.
2. Install it: `adb install -r <apk>`.
3. Run a home server that the phone can reach, and sign in to its web app in a browser.

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
