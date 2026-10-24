# The Outbox

The Outbox holds the emails, SMS messages and calendar invites that TheVault sends for you. Each one is an Outbox item, a note of the built-in `outbox` type. What an agent drafts waits in the Outbox until you approve it. What you write yourself is sent when you select **Ready**.

Open it from **Outbox** in the side menu, under **Inbox**. The number beside it counts the items waiting for your review. The desktop and Android apps show the same page. Everything you do there writes the item's note, which syncs to the home server, and the home server does the sending.

## The page

The page has a tab for each stage of an item:

| Tab | What it lists |
|---|---|
| **Review** | Items an agent drafted, waiting for you to approve or reject them. |
| **Drafts** | Items you started and have not sent. |
| **Sending** | Approved items that the server is sending. An SMS item shows **Waiting for phone** while the phone has not yet sent it. |
| **Sent** | Items that were sent, with a **Sent copy** link once the copy comes back. |
| **Failed** | Items whose send failed, with the reason. |
| **Rejected** | Items you rejected. |

Each item shows a preview of what will be sent and who drafted it:

- An email shows To, Cc, Bcc, Subject, the email it replies to, the body and the attachments.
- An SMS shows the text as a bubble, with the number and the phone that sends it.
- A calendar invite shows each field it sets. For a change to an existing event, each field shows its value now beside the new value.

The actions depend on the item's status:

| Status | Actions |
|---|---|
| Review | **Approve and send**, **Edit**, **Reject** |
| Draft | **Ready**, **Edit**, **Delete** |
| Failed | **Retry**, **Edit** |
| Rejected | **Edit**, **Delete** |

**Edit** opens the item's note, where you can change any field before you send it. **Delete** moves the item to the trash. **Approve all**, at the top of the Review tab, approves every item in Review after you confirm. The confirm says how many items it sends.

**Ready** takes a draft out of Drafts and sends it. The Ready, Approve and send, Reject and Retry buttons are also on the item's own note page.

## The item's page

An item's note page is one form for its kind, with no tabs:

- An email shows To, Cc, Bcc, Subject and the email it replies to, then the body, then the attachments.
- An SMS shows the number and the phone that sends it, then the text.
- A calendar invite shows the event it changes, When, location, calendar and attendees, then the description.

The status is shown at the top and cannot be chosen from a list. The buttons beside the save status move the item: **Ready** on a draft, **Approve and send** and **Reject** on an item in Review, **Retry** on a failed one. Who drafted the item shows below the form, with when it was sent, its sent copy and the reason it failed once it has them.

## Writing an item

Select **New email**, **New SMS** or **New event** on the Outbox page. TheVault creates a draft and opens it. Fill in the fields, then select **Ready** on the note page or in the Drafts tab.

On an email note:

- **Reply** starts a draft reply: To is the sender, Subject is `Re:` and the email's subject, and the body quotes the email. The reply joins the email's Gmail thread.
- **Reply all** starts the same reply to the sender and everyone in the email's To and Cc, leaving out your connected Google address and anyone listed twice. It shows only when the email went to someone besides you.
- **Forward** starts a draft to no one yet: Subject is `Fwd:` and the email's subject, and the body holds a "Forwarded message" header with the email's From, Date, Subject and To, then the email quoted. The email's attachments are attached again. A forward starts a new Gmail thread.

On an SMS message note:

- **Reply** starts an SMS draft to the other person: the sender of a message you received, the recipient of one you sent. When the phone gave a name rather than a number, the number comes from the person's note, their mobile number first.
- **Reply all** shows in a conversation with several people. The phone cannot send a group message, so it starts one SMS draft for each other number and opens the Drafts tab.
- **Forward** starts an SMS draft with no number and the text `Fwd:` and the message.

Each of these creates a draft and opens it, and the device is left empty, so the vault's Android phone sends it.

- **Propose a change** on an event note starts a draft calendar invite for that event. Fill in only the fields to change. An empty field leaves that part of the event as it is.

Your direct edits of an event note do not go through the Outbox. They write back to Google Calendar at once, as before, without emailing the attendees.

## Who may approve

The writer checks every change to an Outbox item by who makes it:

- You may make any change, such as sending a draft, approving or rejecting an item in Review, or retrying a failed one.
- An agent creates items in Review. It may change only the draft and Review items it drafted, and it never approves, sends or rejects one. Turning off an agent's note confirmations does not change this.
- The send handler and the sync tasks only move an approved item through Sending to Sent or Failed.

The exception is an agent whose **Send from the Outbox without review** permission is on: its items are approved as they are created, so they are sent without your review. The permission is off for every agent until you turn it on, on the agent screen's Permissions tab. An agent cannot give another agent this permission unless it has it itself. `docs/ai-agents.md` describes the agent tools that draft items, and `docs/vault-format.md`, "The Outbox", gives the full rules.

## How each kind is sent

The home server sends an item when it becomes approved. The embedded servers in the apps never send: an item approved in an app is sent once the approval syncs to the home server.

### Email

Email goes through Gmail, as the Google account connected in **Settings > Integrations**. TheVault needs Google's `gmail.send` permission for this. A connection made before TheVault could send lacks it: the Google card then says **Reconnect Google in Settings > Integrations to send email.**, and email items fail with that message until you connect again.

Once Gmail has sent the email, TheVault imports Gmail's sent copy as an email note and links it as the item's sent copy. `docs/google.md`, "Sending email from the Outbox", gives the details and the size limits.

### Calendar invites

A calendar invite goes through Google Calendar. TheVault creates the event note, or changes the event the item names, and Google Calendar emails the attendees. The event note is the item's sent copy.

- A new event takes the item's title, **When**, location, attendees, calendar and description. It needs a title and a time. An empty calendar means the default calendar of the Google settings.
- A change sets each field the item holds on the event and leaves the rest as they are. It cannot move an event to another calendar.
- The item is **Sent** once Google Calendar has taken the event, with Google's event id. When Google refuses it, the item is **Failed** with Google's message. **Retry** sends it again, and a new event that failed reuses the event note the first try made.
- When the change asks for nothing the event does not already hold, nothing is sent and the item fails and says so.

Only a write from the Outbox emails the attendees. Your direct edits of an event note write back without emailing anyone.

### SMS

An SMS is sent by your Android phone. The phone must be paired with the vault, and **SMS sending** must be on for it in **Settings > Devices**. Otherwise the item fails and says which setting to turn on. The item's **Device** field picks the phone; empty means the vault's one Android phone with SMS sending on.

The server queues a `send_sms` command for the phone, and the item shows **Waiting for phone** until the phone takes it and sends the message. An SMS carries text only: the body's paragraphs, one per line, without formatting.

- The phone waits up to 60 seconds for Android to say whether the message went. When it went, the item is **Sent**, and its sent reference becomes the phone's id of the sent message (`sms:<id>`).
- When Android reports an error, such as no service, the item is **Failed** with Android's result code. When Android says nothing within 60 seconds, the item is **Failed** too, because the phone cannot tell whether the message went. Check the phone's messages before you select **Retry**.
- The phone's record of the sent message comes back as a message note, linked as the item's sent copy. The link is made by the phone's id of the message. An older app that does not report the id still marks the item **Sent**. Its copy is linked by number and text, to an outgoing SMS sent within 10 minutes of the item.
- A command the phone never takes expires after a day, and the item stays **Sending**. Open the phone app so that it checks for commands.

## Notifications

The bell tells you when the Outbox needs you:

- When an agent drafts items into Review, you get one notification for its turn or run, such as **Clerk drafted 3 items for review**. It opens the Outbox. Items drafted later in the same turn raise the count in place while the notification is unread.
- When a send fails, you get **Could not send** and the item's title, with the reason. It opens the item.
