# Capturing to the inbox

The capture dialog saves a quick thought as a fleeting note in the **Inbox**. Open it from **Capture** in the side menu, the command palette, or <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>K</kbd>. The desktop app also has a global **Quick capture** shortcut (see [The Linux desktop app](desktop.md#quick-capture-and-the-global-hotkey)).

A capture holds any mix of text, images, photos and drawings. **Save**, or <kbd>Ctrl</kbd>+<kbd>Enter</kbd>, writes it as one `fleeting` note: the first line of text is the title, the text and the drawings go in the note's body, and images and photos go in its **Images** field. A capture with no text is titled `Image <date and time>`, or `Drawing <date and time>` when it holds only drawings. <kbd>Esc</kbd> closes the dialog, and asks first when there is something to lose.

## Images

- **Add image** picks image files.
- Paste an image, or drop image files on the dialog.

Each image shows as a thumbnail. Its **×** removes it before you save.

## Camera

**Camera** takes a photo and attaches it as **Add image** does.

- On a phone, in a browser or in the Android app, it opens the phone's camera. Take the photo there, and it is attached when the camera closes. The Android app asks once for permission to use the camera; without it, Camera opens nothing.
- In a desktop browser, it opens a preview in the dialog. **Take photo** takes the picture, **Retake** goes back to the live picture, and **Use photo** attaches it as a JPEG, no larger than 2560 pixels on its long edge. **Cancel**, or <kbd>Esc</kbd>, closes the preview. The browser asks once for permission to use the camera.
- On a computer with no camera the button does not show.

## Pen

**Pen** opens a drawing surface over the dialog. It fills the screen on a phone and most of the window on a desktop. It is the same endless drawing as an ink block in a note (see [Vault file format, Ink](vault-format.md#ink)), with the pen, highlighter, eraser, colours, sizes, backgrounds and undo. Here the mouse and one finger draw from the start; two fingers pan and pinch to zoom, and a stylus draws with its pressure.

- **Done** adds the drawing to the capture. It is disabled until something is drawn.
- **Discard** closes the surface and adds nothing.

Each drawing shows as a small preview in the dialog, and its **×** removes it before you save. A capture can hold several drawings. They are saved as `ink` blocks in the note's body, after the text, and open in the note on the whole drawing.

## Send to an agent

**Send to**, under the text, sends the capture to an agent when it is saved. It lists **No one**, the default, then **Assistant**, then every other enabled agent that a fleeting note may call (see [AI agents in the vault](ai-agents.md)).

With an agent chosen, **Save** adds one more paragraph to the end of the note's body: a call to the agent, `@<Agent> Process this capture`, linked as `agent:<name>?call=<id>` with a new call id. The toast says "Saved and sent to <Agent>". The server answers the call once, as it answers any call written in a note: the agent runs with the note's text as its context, and the note is not changed. The link shows in the note as a chip that opens the agent's thread.

The dialog remembers the choice on each device, so the next capture is sent to the same agent until you choose **No one**. In the apps the list comes from the home server. A call needs a connection to obtain a server-issued id bound to the saved note and agent. Offline, the capture is saved to the inbox without a call and the dialog explains that calling needs a connection. Retrying uses that saved capture rather than creating a duplicate. Once issued, the call runs when the note syncs, provided the unused id has not expired after 24 hours.

## Sharing to TheVault on Android

In another app, choose **Share** and then **TheVault inbox**. The app saves a fleeting note at once, even with no connection, and shows "Saved to TheVault inbox"; the note reaches the home server with the next sync.

- Shared text becomes the note's body. A shared web page is titled with the page title, with its link in the body.
- Shared images go in the note's **Images** field. A file counts as an image when the sharing app, or its file name, says it is one.
- Every other shared file, of any type, such as a PDF, a spreadsheet or an archive, is attached in the body as a file, after the text.
- Several files shared at once make one note with every file attached, whichever way the sharing app hands them over.
- The files are stored in the phone's vault with the note and reach the home server with it. On the server the note shows the same images and files.
- A file the app cannot read saves nothing: the toast says "TheVault could not save this" and why, so you can share it again.

## The inbox

**Inbox** in the side menu lists what waits to be processed, in one list, newest first by each item's own time:

- **Captures**: fleeting notes, by when they were made.
- **Mail**: `email` notes that are not sent mail (no `system/SENT` label), by `sent`.
- **Messages**: incoming `message` notes (`direction` `in`), SMS, MMS and RCS, by `sent`.
- **Calls**: missed `call` notes (`direction` `missed`), by `started`.

Each counts only while its `processed` field is not ticked; a note with no value counts as unprocessed. The chips **All**, **Captures**, **Mail**, **Messages** and **Calls** narrow the list to one kind. It shows 50 items, and **Show more** adds 50. **Show processed** lists the processed ones too, with the same rules for sent mail, outgoing messages and calls that were not missed. The number beside **Inbox** in the side menu counts what **All** lists.

Each row has **Open** and **Done**. **Done** ticks `processed`, and the row leaves the list; the toast's **Undo**, or **Back to inbox** under **Show processed**, clears it again. A capture also has **Convert to…**, which makes a note of another type from it and deletes the fleeting note. Every row but a mail has **Delete**, which moves the note to the trash; a mail has none, because the trash removes only the vault's copy and not the mail in Gmail.

The desktop and Android apps show the same inbox and answer it from their local vault, so it works with no connection.

### Mark older items processed

A vault with years of mail would start with all of it in the inbox. **Mark older processed…**, in the inbox's header, clears it once: pick a date, and the dialog counts the emails, messages and calls sent or made before it that are not processed yet, by kind. **Mark N items processed** ticks `processed` on each of them; nothing else in the notes changes, and nothing is sent to Gmail. Captures are never touched, and an item with no date is left alone. Running it again finds nothing more, since it counts only what is not processed.

The home server does the work, in batches of 200 notes, each one commit, so syncing and other edits go on between batches (`POST /api/inbox/mark-processed`, which needs an admin session or token). The apps do not offer it: run it in the web app, and the change reaches the apps with the next sync.
