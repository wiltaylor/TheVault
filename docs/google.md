# Connecting Gmail and Google Calendar

TheVault syncs Gmail and Google Calendar from the home server, and sends the email you approve in the Outbox through Gmail. The server signs in to Google with an OAuth client that you create in your own Google Cloud project, so no third party sits between your vault and your Google account.

This guide covers the one-time setup in Google Cloud, connecting from Settings > Integrations > Google, and what keeps the sign-in working afterwards.

## What you need

- A Google account with Gmail or Google Calendar.
- Access to the [Google Cloud console](https://console.cloud.google.com/) with that account. A Google Cloud project for personal use costs nothing.
- The TheVault web app, signed in.

## Choose the client type

Google lets an OAuth client send the browser back to two kinds of address. Which one you use depends on how you open TheVault.

| You open TheVault at | Client type | How you finish signing in |
|---|---|---|
| An `http://` address, such as `http://100.81.94.109:8787` on Tailscale | **Desktop app** | You paste an address back into TheVault |
| An `https://` address, such as `https://vault.tailnet-name.ts.net` or a reverse proxy | **Web application** | Google sends you straight back to TheVault |

Google refuses plain `http` redirect addresses and raw IP addresses, so a server reached at `http://<ip>:8787` cannot receive Google's redirect. A Desktop app client works around this: Google sends the browser to `http://127.0.0.1:53682/`, the page fails to load, and you copy that address into TheVault. The address holds a one-time code that TheVault exchanges for the sign-in.

The Integrations page picks the flow from the page's own address: `https:` uses the direct flow, anything else uses the paste flow. You can create both clients and use whichever matches the address you open.

## Set up Google Cloud

### 1. Create a project

1. Open the [Google Cloud console](https://console.cloud.google.com/).
2. Open the project picker at the top of the page and select **New project**.
3. Name it, for example `TheVault`, and select **Create**.
4. Make sure the new project is selected in the project picker before you continue.

### 2. Enable the Gmail and Calendar APIs

1. Go to **APIs & Services > Library**.
2. Search for **Gmail API**, open it and select **Enable**.
3. Search for **Google Calendar API**, open it and select **Enable**.

### 3. Configure the consent screen

1. Go to **Google Auth Platform** (in older consoles, **APIs & Services > OAuth consent screen**) and select **Get started** if the project has no consent screen yet.
2. Enter an app name, such as `TheVault`, and your address as the user support email.
3. For **Audience**, choose **External**. A personal Gmail account can only choose External.
4. Enter your address as the developer contact and finish the wizard.

You do not need to add scopes on the consent screen. TheVault asks for them when you connect.

### 4. Set the publishing status to In production

This step decides whether the sign-in lasts.

1. In **Google Auth Platform > Audience**, find **Publishing status**.
2. If it says **Testing**, select **Publish app** and confirm. The status changes to **In production**.

While a project is in **Testing**, Google expires every refresh token for Gmail and Calendar scopes 7 days after consent. TheVault would lose access every week. An app **In production** does not have that limit.

You do not need to submit the app for verification. Google lets an unverified app be used by its developer and up to 100 people in total, which covers personal use. The cost is a warning screen when you sign in (see [Connect TheVault](#connect-thevault)).

### 5. Create the OAuth client

1. Go to **Google Auth Platform > Clients** (or **APIs & Services > Credentials**) and select **Create client** (or **Create credentials > OAuth client ID**).
2. For **Application type**, choose the type from [Choose the client type](#choose-the-client-type):
   - **Desktop app**: name it and select **Create**. A Desktop client has no redirect addresses to register.
   - **Web application**: name it, then under **Authorized redirect URIs** add your TheVault address followed by `/integrations/google/callback`, for example `https://vault.tailnet-name.ts.net/integrations/google/callback`. The address must match exactly, including `https` and any port. Select **Create**.
3. Copy the **Client ID** and **Client secret**. You can see them again later under **Clients**.

## Connect TheVault

1. In TheVault, open **Settings > Integrations**.
2. In the **Google** card, enter the **Client ID** and **Client secret**.
3. Select **Connect Google**.
4. Sign in to Google and continue past the warning:
   1. Google shows **Google hasn't verified this app**. This is expected for your own unverified client.
   2. Select **Advanced**, then **Go to TheVault (unsafe)**. The name is your project's app name.
   3. Tick every permission TheVault asks for and select **Continue**.
5. Finish the sign-in:
   - **Paste flow** (http): TheVault shows an **Open Google's sign-in page** link. After you approve, the browser goes to a `http://127.0.0.1:53682/?state=…&code=…` page that does not load. Copy the whole address from the address bar, paste it into **Address from the browser** and select **Finish connecting**. Do this within a few minutes: the code is short-lived and works once.
   - **Direct flow** (https): Google sends the browser back to TheVault, which finishes on its own.

The card then shows **Connected as** your Google address.

### What connecting changes

- TheVault stores three secrets, encrypted with the vault's secret key: `google_client_id`, `google_client_secret` and `google_refresh_token`. Settings > Secrets lists them; no page or API shows their values.
- TheVault stores the connected address as the secret `google_account`, and the sync settings as the secret `google_settings`. No note holds either.
- The scheduled tasks `google_gmail` and `google_calendar` are unpaused, and each runs once straight away. The Integrations card shows **Syncing…** for each side while its run is going; a first run can take minutes. When a run ends, the card shows its time, its summary line (for example `Gmail: 12 new, 0 relabelled, 0 archived, 0 not saved.`) and the last error, with **Run now** buttons. **Run now** answers as soon as the run starts; the card shows **Syncing…** and reads the run until it ends, then says how it went.

## Choose what to sync

The Google card's **What to sync** form holds the sync's settings. It is the only place to change them: they are kept as the secret `google_settings`, not as a note. Select **Save** after a change; the next run of each sync uses it.

| Setting | What it does | Default |
|---|---|---|
| **Gmail mode** | **The labels below** pulls only the Gmail labels listed; **All mail** pulls everything | The labels below |
| **Gmail labels** | The labels to pull, by name, such as `INBOX` or `Work/Clients`. Type a label and press Enter to add it | None |
| **Window (days)** | How many days back the first Gmail pull reaches, 1 to 3650 | 90 |
| **Back-fill to** | Pull older mail, one page each run, back to this date. Leave it empty to pull nothing older than the window | Empty |
| **Calendars** | The ids of the Google calendars to pull. `primary` is your main calendar; another calendar's id is under its **Integrate calendar** settings in Google Calendar | `primary` |
| **Default calendar** | Where a new event note goes when it names no calendar | `primary` |

Each sync also keeps its progress, such as Gmail's history id and each calendar's sync token, in a secret of its own: `google_gmail_state` and `google_calendar_state`. A sync run writes no note history for them. `docs/scripts.md`, "The Google sync's secrets", describes every Google secret.

A vault from 0.1.7 had a **Google sync** settings note instead. The first start of a newer server moves its settings and progress into these secrets and deletes the note and its type. That needs the vault's secret key; a server started without it keeps the note and moves it on a later start that has the key.

## The permissions TheVault asks for

| Scope | Why |
|---|---|
| `https://www.googleapis.com/auth/gmail.readonly` | Read mail and labels. TheVault never deletes or changes mail. Google classes this scope as restricted; no narrower scope can read message bodies. |
| `https://www.googleapis.com/auth/gmail.send` | Send the emails you approve in the Outbox. See [Sending email from the Outbox](#sending-email-from-the-outbox). |
| `https://www.googleapis.com/auth/calendar` | Read events, and write back the events you edit, create or delete in the vault. |
| `openid`, `email` | Learn which Google address signed in, to show it on the card. |

TheVault asks with `access_type=offline` and `prompt=consent`, so Google issues a refresh token every time you connect, and uses PKCE to bind the code to the server that asked for it.

When you connect, TheVault records the scopes Google granted in the `google_settings` secret, as `scopes`. Google's consent screen lets you untick a permission, so TheVault checks this list rather than assume it has every scope it asked for.

## Sending email from the Outbox

TheVault sends an email only when an Outbox item of kind `email` becomes `approved`. That happens when you select **Send** on an item you wrote, or **Approve and send** on an item an agent drafted. An agent's drafts wait in Review for you. The exception is an agent whose permissions turn on **Send from the Outbox without review**: its items go straight to `approved`. That switch is off for every agent unless you turn it on.

The home server sends the item through Gmail as your connected address. An item approved in the desktop or Android app is sent once the approval syncs to the server.

1. The item's status moves to **Sending**.
2. TheVault builds the message from the item: To, Cc, Bcc, Subject, the body as plain text and as HTML, and the attachments. A reply carries `In-Reply-To` and `References` from the email it answers, and joins that email's Gmail thread.
3. TheVault sends the message with `messages.send`. A message of 4.5 MB or more is sent through Gmail's upload endpoint instead.
4. The item's status moves to **Sent**, with the time and Gmail's message id.
5. TheVault imports the sent copy from Gmail as an email note and links it to the item as its sent copy. The `google_gmail` sync also links any copy that did not arrive at once, whatever labels it syncs.

Attachments come to at most 25 MB together, Gmail's limit, and at most 16 MB each, the most a script can read. An item over either limit fails before anything is sent, and the error says which limit it hit.

When a send fails, the item's status moves to **Failed** with the reason, such as Gmail's error message. Fix the item and approve it again to retry.

### Reconnect to send

A connection made before TheVault could send has no `gmail.send` scope. The Google card then shows **Reconnect Google in Settings > Integrations to send email.** with a **Connect** button, and an email item fails with the same message. To fix it:

1. Open **Settings > Integrations**.
2. In the **Google** card, select **Connect**.
3. Enter the client ID and secret again and select **Connect Google**.
4. Tick every permission on Google's consent screen, including **Send email on your behalf**, and finish connecting.

The card stops showing the line once Google has granted the scope. `GET /api/integrations/google` answers `can_send: true` from then on.

## Keeping the sign-in alive

Google does not rotate refresh tokens: TheVault keeps the one it got when you connected. The sync refreshes an access token on every run, which also keeps the token and the client in use.

A refresh token stops working when any of these happen:

- The project's publishing status is **Testing**: the token expires 7 days after consent. Set it to **In production**.
- You remove TheVault's access in your Google account's security settings.
- The token is not used for six months, for example because both sync tasks were paused that long.
- You change your Google password. This ends tokens that carry Gmail scopes.
- You connect more than 100 times with the same client. Google allows 100 live refresh tokens per account per client and silently ends the oldest when a new one is issued.
- Google deletes the OAuth client after six months without token requests or setting changes. Google emails the project owner 30 days before.
- You granted time-limited access on the consent screen and the time ran out.
- A Google Workspace administrator restricted access. This does not apply to a personal account.

When this happens, the next sync run gets `invalid_grant` from Google. TheVault adds one bell notification, **Google sign-in expired**, and the Integrations card shows the error for Gmail and Calendar. Later runs fail quietly until you reconnect. To fix it, open **Settings > Integrations**, enter the client ID and secret again and select **Connect Google**.

## Disconnect

Select **Disconnect** on the Google card. TheVault deletes the three sign-in secrets and the connected address, forgets the granted scopes, pauses both sync tasks, and clears each sync's progress, so the next connection pulls from the start. The Outbox cannot send email until you connect again. The **What to sync** settings stay. Notes that were already synced stay in the vault. To stop Google from accepting the old token as well, remove TheVault's access in your Google account's security settings.

## Troubleshooting

| Message | Cause and fix |
|---|---|
| **Google answered invalid_client** | The client ID or secret is wrong, or the client was deleted. Copy both again from Google Cloud. |
| **Google answered redirect_uri_mismatch** | Web client: the authorized redirect URI does not match the page's address exactly. Add `<your address>/integrations/google/callback`. Desktop client on an https page: use a Web application client, or open TheVault on its http address. |
| **Google answered invalid_grant** while connecting | The code was already used or expired. Select **Connect Google** again and paste promptly. |
| **this sign-in was not started here, or it expired or was already used** | The pasted address belongs to an older attempt, or more than 10 minutes passed, or the server restarted. Select **Connect Google** again. |
| **Error 403: access_denied** on Google's page | The project is in Testing and your account is not a listed test user. Set the publishing status to **In production**. |
