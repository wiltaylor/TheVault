# The Outbox

The Outbox holds the emails, SMS messages and calendar invites that TheVault is about to send for you. Each one is an Outbox item, a note of the built-in `outbox` type. An item is a draft until you select **Ready**, whether you wrote it or an agent drafted it. Once it is sent, it leaves the Outbox: what was sent is kept as a regular email, message or event note.

Open it from **Outbox** in the side menu, under **Inbox**. The number beside it counts the drafts agents made, which wait for you. The desktop and Android apps show the same page. Everything you do there writes the item's note, which syncs to the home server, and the home server does the sending.

## Statuses

An item has one of four statuses:

| Status | Meaning |
|---|---|
| `draft` | Not sent. It waits for you to select **Ready**. |
| `ready` | You selected **Ready**. The home server sends it next. |
| `sending` | The home server is sending it, or it was sent and TheVault waits for the sent copy. |
| `failed` | The send failed. The item shows the reason. |

There is no sent status. When a send succeeds and the sent copy is in the vault, TheVault removes the item for good: it is not moved to the trash, and its history is removed with it. The sent copy is the record of what was sent, and the Log keeps an `outbox.sent` entry with the kind and the recipients. An item whose copy does not come back within 24 hours of the send is removed anyway.

## The page

The page has three tabs:

| Tab | What it lists |
|---|---|
| **Draft** | Items not sent yet: the ones you started and the ones agents drafted. |
| **Ready** | Items marked ready and items being sent. An SMS item shows **Waiting for phone** while the phone has not sent it yet. |
| **Error** | Items whose send failed, with the reason. |

Each item shows a preview of what will be sent:

- An email shows To, Cc, Bcc, Subject, the email it replies to, the body and the attachments.
- An SMS shows the text as a bubble, with the number and the phone that sends it.
- A calendar invite shows each field it sets. For a change to an existing event, each field shows its value now beside the new value.

Beside the item's kind, a small chip shows its status. An item an agent drafted has a second chip, **AI**. Point at it to see which agent drafted the item. The items you drafted have no second chip.

The actions depend on the item's status:

| Status | Actions |
|---|---|
| Draft | **Ready**, **Edit**, **Delete** |
| Ready, sending | None |
| Failed | **Retry**, **Edit**, **Delete** |

**Ready** sends the draft. **Retry** sends a failed item again. **Edit** opens the item's note, where you can change any field before you send it. **Delete** moves the item to the trash, where you can restore it. To reject a draft an agent made, delete it.

## The item's page

An item's note page is one form for its kind, with no tabs:

- An email shows To, Cc, Bcc, Subject and the email it replies to, then the body, then the attachments.
- An SMS shows the number and the phone that sends it, then the text.
- A calendar invite shows the event it changes, When, location, calendar and attendees, then the description.

The status is shown at the top as a chip, with the **AI** chip beside it for an agent's draft, and cannot be chosen from a list. The buttons beside the save status act on the item: **Ready** and **Delete** on a draft, **Retry** and **Delete** on a failed item. The reason a send failed shows below the form.

## Writing an item

Select **New email**, **New SMS** or **New event** on the Outbox page. TheVault creates a draft and opens it. Fill in the fields, then select **Ready** on the note page or in the Draft tab.

On an email note:

- **Reply** starts a draft reply: To is the sender, Subject is `Re:` and the email's subject, and the body quotes the email. The reply joins the email's Gmail thread.
- **Reply all** starts the same reply to the sender and everyone in the email's To and Cc, leaving out your connected Google address and anyone listed twice. It shows only when the email went to someone besides you.
- **Forward** starts a draft to no one yet: Subject is `Fwd:` and the email's subject, and the body holds a "Forwarded message" header with the email's From, Date, Subject and To, then the email quoted. The email's attachments are attached again. A forward starts a new Gmail thread.

On an SMS message note:

- **Reply** starts an SMS draft to the other person: the sender of a message you received, the recipient of one you sent. When the phone gave a name rather than a number, the number comes from the person's note, their mobile number first.
- **Reply all** shows in a conversation with several people. The phone cannot send a group message, so it starts one SMS draft for each other number and opens the Draft tab.
- **Forward** starts an SMS draft with no number and the text `Fwd:` and the message.

Each of these creates a draft and opens it, and the device is left empty, so the vault's Android phone sends it.

- **Propose a change** on an event note starts a draft calendar invite for that event. Fill in only the fields to change. An empty field leaves that part of the event as it is.

Your direct edits of an event note do not go through the Outbox. They write back to Google Calendar at once, as before, without emailing the attendees.

## Who may send

The writer checks every change to an Outbox item by who makes it:

- You may make any change, such as marking a draft ready or retrying a failed item.
- An agent creates items as drafts. It may change only the drafts it made, and they stay drafts: it never marks an item ready. Turning off an agent's note confirmations does not change this.
- The send handler and the sync tasks only move a ready item to sending and then to failed, and remove an item once it was sent.

The exception is an agent whose **Send from the Outbox without review** permission is on: its items are created ready, so they are sent without waiting for you. The permission is off for every agent until you turn it on, on the agent screen's Permissions tab. An agent cannot give another agent this permission unless it has it itself. `docs/ai-agents.md` describes the agent tools that draft items, and `docs/vault-format.md`, "The Outbox", gives the full rules.

## How each kind is sent

The home server sends an item when it becomes ready. The embedded servers in the apps never send: an item marked ready in an app is sent once the change syncs to the home server.

### Email

Email goes through Gmail, as the Google account connected in **Settings > Integrations**. TheVault needs Google's `gmail.send` permission for this. A connection made before TheVault could send lacks it: the Google card then says **Reconnect Google in Settings > Integrations to send email.**, and email items fail with that message until you connect again.

Once Gmail has sent the email, TheVault imports Gmail's sent copy as an email note and removes the item. When the copy cannot be fetched at once, the next Gmail sync imports it. `docs/google.md`, "Sending email from the Outbox", gives the details and the size limits.

### Calendar invites

A calendar invite goes through Google Calendar. TheVault creates the event note, or changes the event the item names, and Google Calendar emails the attendees. The event note is the sent copy.

- A new event takes the item's title, **When**, location, attendees, calendar and description. It needs a title and a time. An empty calendar means the default calendar of the Google settings.
- A change sets each field the item holds on the event and leaves the rest as they are. It cannot move an event to another calendar.
- The item is removed once Google Calendar has taken the event. When Google refuses it, the item is **Failed** with Google's message. **Retry** sends it again, and a new event that failed reuses the event note the first try made.
- When the change asks for nothing the event does not already hold, nothing is sent and the item fails and says so.

Only a write from the Outbox emails the attendees. Your direct edits of an event note write back without emailing anyone.

### SMS

An SMS is sent by your Android phone. The phone must be paired with the vault, and **SMS sending** must be on for it in **Settings > Devices**. Otherwise the item fails and says which setting to turn on. The item's **Device** field picks the phone; empty means the vault's one Android phone with SMS sending on.

The server queues a `send_sms` command for the phone, and the item shows **Waiting for phone** until the phone takes it and sends the message. An SMS carries text only: the body's paragraphs, one per line, without formatting.

- The phone waits up to 60 seconds for Android to say whether the message went. When it went, the item records the send, and its sent reference becomes the phone's id of the sent message (`sms:<id>`).
- When Android reports an error, such as no service, the item is **Failed** with Android's result code. When Android says nothing within 60 seconds, the item is **Failed** too, because the phone cannot tell whether the message went. Check the phone's messages before you select **Retry**.
- The phone's record of the sent message comes back as a message note, and the item is then removed. The copy is found by the phone's id of the message. An older app that does not report the id still records the send; its copy is found by number and text, an outgoing SMS sent within 10 minutes of the item.
- When the copy never comes back, the item stays **Sending** and is removed 24 hours after the send.
- A command the phone never takes expires after a day, and the item stays **Sending**. Open the phone app so that it checks for commands.

## Notifications

The bell tells you when the Outbox needs you:

- When an agent drafts items, you get one notification for its turn or run, such as **Clerk drafted 3 items**. It opens the Draft tab. Items drafted later in the same turn raise the count in place while the notification is unread.
- When a send fails, you get **Could not send** and the item's title, with the reason. It opens the item.

## Upgrading a vault

A vault from before these four statuses is moved to them once, by upgrade step `0009_outbox_statuses`, when the server starts: an item in Review becomes a draft, an approved one becomes ready, a rejected one moves to the trash, and a sent one is removed, since its copy is already in the vault. Drafts, items being sent and failed items stay as they are. A vault whose `outbox` type has status options you changed is left alone.
