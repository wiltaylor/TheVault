# The Outbox

The Outbox holds the emails, SMS messages and calendar invites that TheVault sends for you. Each one is an Outbox item, a note of the built-in `outbox` type. What an agent drafts waits in the Outbox until you approve it. What you write yourself is sent when you select **Send**.

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
| Draft | **Send**, **Edit**, **Delete** |
| Failed | **Retry**, **Edit** |
| Rejected | **Edit**, **Delete** |

**Edit** opens the item's note, where you can change any field before you send it. **Delete** moves the item to the trash. **Approve all**, at the top of the Review tab, approves every item in Review after you confirm. The confirm says how many items it sends.

The Send, Approve and send and Retry buttons are also on the item's own note page.

## Writing an item

Select **New email**, **New SMS** or **New event** on the Outbox page. TheVault creates a draft and opens it. Fill in the fields, then select **Send** on the note page or in the Drafts tab.

- **Reply** on an email note starts a draft reply: To is the sender, Subject is `Re:` and the email's subject, and the body quotes the email. The reply joins the email's Gmail thread.
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

### SMS

An SMS is sent by your Android phone. The phone must be paired with the vault, and **SMS sending** must be on for it in **Settings > Devices**. Otherwise the item fails and says which setting to turn on. The item's **Device** field picks the phone; empty means the vault's one Android phone with SMS sending on.

The server queues a command for the phone, and the item shows **Waiting for phone** until the phone takes it and sends the message. The phone's record of the sent message comes back as a message note, linked as the item's sent copy. An SMS carries text only.

Calendar invites and SMS sending arrive in later releases. Until then, an item of either kind fails with a message that says it is not available yet.

## Notifications

The bell tells you when the Outbox needs you:

- When an agent drafts items into Review, you get one notification for its turn or run, such as **Clerk drafted 3 items for review**. It opens the Outbox. Items drafted later in the same turn raise the count in place while the notification is unread.
- When a send fails, you get **Could not send** and the item's title, with the reason. It opens the item.
