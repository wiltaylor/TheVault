# Capturing to the inbox

The capture dialog saves a quick thought as a fleeting note in the **Inbox**. Open it from **Capture** in the side menu, the command palette, <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>K</kbd>, or the round **+** button on a phone. The desktop app also has a global **Quick capture** shortcut (see [The Linux desktop app](desktop.md#quick-capture-and-the-global-hotkey)).

A capture holds any mix of text, images, photos and drawings. **Save**, or <kbd>Ctrl</kbd>+<kbd>Enter</kbd>, writes it as one `fleeting` note: the first line of text is the title, the text and the drawings go in the note's body, and images and photos go in its **Images** field. A capture with no text is titled `Image <date and time>`, or `Drawing <date and time>` when it holds only drawings. <kbd>Esc</kbd> closes the dialog, and asks first when there is something to lose.

## Images

- **Add image** picks image files.
- Paste an image, or drop image files on the dialog.

Each image shows as a thumbnail. Its **×** removes it before you save.

## Camera

**Camera** takes a photo and attaches it as **Add image** does.

- On a phone, in a browser or in the Android app, it opens the phone's camera. Take the photo there, and it is attached when the camera closes.
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

The dialog remembers the choice on each device, so the next capture is sent to the same agent until you choose **No one**. In the apps the list comes from the home server. With no connection the app still saves the call, and the server answers it when the note syncs.

## Sharing to TheVault on Android

In another app, choose **Share** and then **TheVault inbox**. The app saves a fleeting note at once, even with no connection, and shows "Saved to TheVault inbox"; the note reaches the home server with the next sync.

- Shared text becomes the note's body. A shared web page is titled with the page title, with its link in the body.
- Shared images go in the note's **Images** field.
- Other shared files, such as PDFs, are attached in the body as files.
- Several files shared at once make one note with every file attached.
