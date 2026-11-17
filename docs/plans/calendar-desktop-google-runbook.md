# Fabric Calendar: Google project runbook

**What this is.** The steps the owner does himself in Google's consoles so that Fabric Calendar can be developed and
tested against real Google Calendar accounts. It is a deliverable of work package `CD-W05` of the
[Calendar Desktop plan](calendar-desktop-plan.md) (section 10.1, decisions C08 and Q08).

**Who does it.** The owner, by hand, in a browser. No session does any step of this document. A session never signs
in to Google, never creates an account, a project or a client, and never reads the client file. The only thing a
session is given is the path of the client file (step 6.4).

**What it is for.** Development and gate `CD-G2b` only. The project stays in Google's testing mode. Publishing the
app, Google's verification and the branding a release needs are work package `CD-W11` and question Q08.

Date 2026-11-17. Status: Proposed, for the owner to follow and correct. Every console step was compared with
Google's documentation on that date. Section 9 says, step by step, what Google's pages confirm and what they do not.
Google changes its console often: where a label on your screen differs from this document, trust the screen and tell
the session what differed.

---------------------------------------------------------------------------------------------------------------------

## 0. What you have at the end

| # | Thing | Used for |
|---|---|---|
| 1 | One Google account that only administers the project (the admin account) | Owns the Cloud project. It is not a test account. |
| 2 | One Google Cloud project with the Google Calendar API enabled | Every Calendar call counts against this project. |
| 3 | The consent screen: app name Fabric Calendar, user type External, publishing status Testing, two test users, five scopes | What the test accounts see when they connect. |
| 4 | One OAuth client of type Desktop app, and its client file saved outside the repository | The broker's identity towards Google. |
| 5 | Two disposable Gmail accounts, A and B | The accounts Fabric Calendar connects in tests. |
| 6 | In each test account: a calendar it owns, shared with write access to the other account | Acceptance needs an owned and a writable shared calendar per account (CD-W00b). |
| 7 | A calendar named Agent Drafts in account A | The owner creates it; Fabric never creates a calendar (CD-Q05f). |
| 8 | One mailbox you own that may receive test invitations | The only outside recipient a test may invite. |
| 9 | Five test events made by hand (section 8) | The rows of gate `CD-G2b` that need existing data. |

Nothing in this list costs money. If any page asks for a billing account or a payment method, stop and tell the
session (see step 2.3).

---------------------------------------------------------------------------------------------------------------------

## 1. The three Google accounts

1.1 **The admin account.** Choose or create one Google account that will own the Cloud project and do nothing else.
Do not use test account A or B for this: a test account is disposable and the project must outlive it.

1.2 Turn on 2-Step Verification for the admin account. Google requires it to sign in to the Cloud console, and
enforces it account by account; the console shows a reminder before it does.

1.3 **Test accounts A and B.** Create two new Gmail accounts used for nothing else. They hold only test events.
Write their addresses down for yourself. They do not go into the repository, and a session does not need them: the
broker learns an account's address from Google when you connect it.

1.4 **The invitation mailbox.** Decide which mailbox you own may receive test invitations. Nothing is set up for it
here. A mailbox outside Google lets you also see what `send_updates: external_only` does, because Google documents
that value as "sent to non-Google Calendar guests only".

---------------------------------------------------------------------------------------------------------------------

## 2. The Google Cloud project

Sign in to the Google Cloud console (`https://console.cloud.google.com/`) as the admin account.

2.1 Go to Menu > IAM & Admin > Create a Project.

2.2 In Project Name enter a name, for example `Fabric Calendar Dev`. Google then shows a project ID under the name.
The project ID cannot be changed later. In Location keep what the console proposes (for a personal account that is
no organization) unless you want the project inside an organization you own. Click Create. Google says the project is created "within a few minutes".

2.3 Do not add a billing account. The Calendar API has no charge at these volumes. If the console asks for billing
at any step, stop there and tell the session: the plan has no paid service, and adding one is your decision alone.

2.4 Make sure the new project is the one selected in the project picker at the top of the console before every step
below.

---------------------------------------------------------------------------------------------------------------------

## 3. Enable the Calendar API

3.1 Go to Menu > APIs & Services > Library > Google Workspace.

3.2 Click Google Calendar API.

3.3 Click Enable.

Enable nothing else. Google People import is a later work package with its own scope and its own step here
(CD-Q06). Gmail is not part of this product version.

---------------------------------------------------------------------------------------------------------------------

## 4. The consent screen

Google now calls this area Google Auth Platform. It has the pages Branding, Audience, Data Access and Clients.

4.1 Go to Menu > Google Auth platform > Branding. If the page says "Google Auth platform not configured yet", click
Get Started.

4.2 Under App Information: App name `Fabric Calendar`. User support email: an address of yours that a test user
could write to. Google shows this address on the consent screen.

4.3 Under Audience choose **External**. Internal is only for accounts of one Google Workspace organization and
would refuse your Gmail test accounts.

4.4 Under Contact Information enter the developer contact email: an address where Google can tell you about changes
to the project. It may be the same as the support email.

4.5 Read the Google API Services User Data Policy, tick "I agree to the Google API Services: User Data Policy", and
click Create. Agreeing to that policy is your act as the project's owner; no session does it for you.

4.6 Open the Audience page. Check that **Publishing status** says **Testing**. Do not click Publish app. Section 7
says why.

4.7 Still on the Audience page, under Test users click Add users. Enter the addresses of test account A and test
account B. Click Save. Only accounts on this list can connect while the status is Testing. A project in testing may
list at most 100 test users, and an address counts against that number once it was added.

---------------------------------------------------------------------------------------------------------------------

## 5. The scopes

5.1 Go to Data Access > Add or Remove Scopes.

5.2 Add exactly these five scopes, then click Save. They are the candidate scopes of plan section 10.2.

| Scope | Why Fabric Calendar asks for it |
|---|---|
| `openid` | To learn which Google account connected, by its stable account id. |
| `https://www.googleapis.com/auth/userinfo.email` (shown as `email`) | To show the account's verified address. |
| `https://www.googleapis.com/auth/calendar.calendarlist.readonly` | To list the calendars the account has, so you can select some. |
| `https://www.googleapis.com/auth/calendar.freebusy` | To read busy times without event details. |
| `https://www.googleapis.com/auth/calendar.events` | To read, create, change and delete events on the calendars the account can reach, its own and those shared with it (C13). |

5.3 Never add these, here or later:

- `https://www.googleapis.com/auth/calendar`, the blanket scope. It also allows deleting and sharing whole calendars.
- `https://www.googleapis.com/auth/calendar.acls` or `calendar.acls.readonly`. Fabric never reads or changes sharing.
- `calendar.calendars`, `calendar.calendarlist` (without `.readonly`), `calendar.app.created`,
  `calendar.settings.readonly`, and anything of Gmail, Drive or People.

5.4 The scope list is not final. Plan section 10.2 freezes it after the test on personal accounts (gate `CD-G2b`).
If that test shows a scope is missing or too wide, this section changes and you repeat step 5.2.

A scope on this page only declares what the app may ask for. What an account really granted is decided on the
consent screen, where a person can untick single scopes. Fabric Calendar checks what was granted each time.

---------------------------------------------------------------------------------------------------------------------

## 6. The OAuth client

6.1 Go to Menu > Google Auth platform > Clients. Click Create Client.

6.2 Application type: **Desktop app**. Name: for example `Fabric Calendar desktop (dev)`. The name is shown only in
the console. A Desktop app client needs no redirect address: Fabric Calendar listens on a local port of this Mac
while you sign in.

6.3 **Open choice, yours to make: "Use this client for an AI-powered agent".** Google's page offers this as an
optional switch and says: "If your app is an AI agent that takes actions on behalf of users, select Use this client
for an AI-powered agent", and "You can't change this setting after you create the client." Google requires it only
for clients that call Google's own hosted Workspace MCP servers. Fabric Calendar calls the Calendar API directly and
is the broker a person uses to control agents, not an agent. Recommendation: leave it **off** for this development
client. If you later decide otherwise, you create a second client; this one cannot be switched. Nothing in the
repository records a decision on this yet, so the session has asked for one.

6.4 Click Create. Google now shows the client ID and the client secret. **Download the client file (JSON) in this
dialog, now.** Google shows and lets you download the full secret only once, when the client is created; afterwards
the console shows only its last four characters. If you miss it, open the client, click Add Secret, download the
new one, and disable the old one.

6.5 Save the file outside every repository and outside any folder that syncs to a cloud drive. A suggestion:

```sh
mkdir -p "$HOME/.fabric-calendar-dev" && chmod 700 "$HOME/.fabric-calendar-dev"
mv "$HOME/Downloads/client_secret_<...>.json" "$HOME/.fabric-calendar-dev/google-oauth-client.json"
chmod 600 "$HOME/.fabric-calendar-dev/google-oauth-client.json"
```

6.6 Give the session only the **path** of that file. Do not paste its content into a chat, an issue, a pull request
or a commit. The secret of a desktop client is not a security boundary (Google: desktop apps "cannot keep secrets"),
but a published one lets anyone show a consent screen under your app's name, and it would have to be replaced.

6.7 Google deletes an OAuth client that was not used for six months, after a warning email 30 days before. If
development pauses that long, expect to create the client again.

---------------------------------------------------------------------------------------------------------------------

## 7. What testing mode means

- **Connections last seven days.** Google: "Authorizations by a test user will expire seven days from the time of
  consent", and a refresh token issued to the app "will also expire". So during development each test account must
  be connected again at least once a week. Fabric Calendar must then show "sign in again", not an error, and no
  grant may come back to life by itself. That behaviour is one of the rows of gate `CD-G2b`. The seven days do not
  apply to an app that asks only for `openid` and the address, but Fabric Calendar asks for calendar scopes.
- **A warning screen.** Before the consent screen, Google warns a test user that the app is not verified. On your
  own test accounts you continue past it.
- **Only listed accounts.** An account that is not a test user of the project is refused.
- **Do not click Publish app.** In production an app that asks for scopes Google classes as sensitive needs
  Google's verification before the warning goes away, and until then at most 100 accounts in total may ever grant
  it; that count cannot be reset. Calendar scopes are very likely in that class (section 9). Publishing is a
  release step (`CD-W11`, Q08), not a development step.
- **Other limits of a refresh token**, which hold in any mode: Google ends a refresh token that was not used for six
  months, one whose account removed the app's access, and the oldest ones once an account holds more than 100 for
  the same client. Fabric Calendar treats every one of these as "sign in again".

---------------------------------------------------------------------------------------------------------------------

## 8. Calendars and test data in the two test accounts

Do this in Google Calendar in a browser on a computer (`https://calendar.google.com/`). Google says a new calendar
can only be created from a browser. Use a separate browser profile or private window per account so the two do not
mix.

### 8.1 Calendars

| Step | Signed in as | Do |
|---|---|---|
| 8.1.1 | A | On the left, next to "Other calendars", click Add other calendars > Create new calendar. Name `Fabric Test Shared A`. Click Create calendar. |
| 8.1.2 | A | Open that calendar's Settings and sharing. Under "Shared with" click Add people and groups, enter B's address, choose **Make changes and see event details**, click Send. |
| 8.1.3 | B | Open the email from Google and click the link in it to add the calendar. Without this click the calendar does not appear in B's list. |
| 8.1.4 | B | Create a calendar `Fabric Test Shared B` the same way and share it with A, again **Make changes and see event details**. |
| 8.1.5 | A | Open the email and click the link to add `Fabric Test Shared B`. |
| 8.1.6 | A | Create one more calendar named `Agent Drafts`. Do not share it. |

Choose "Make changes and see event details", not "Make changes and manage sharing": Fabric Calendar never needs the
right to change sharing, and the tests should show that write access alone is enough. Do not choose "Make changes
(see private events as free/busy)"; that is a third kind of write access, noted in section 10.

After this, account A sees: its own main calendar, `Fabric Test Shared A` (owned), `Fabric Test Shared B` (may
write), `Agent Drafts` (owned). Account B sees its own main calendar, `Fabric Test Shared B` (owned) and
`Fabric Test Shared A` (may write).

### 8.2 The five test events

Create them by hand, with these titles, on days within the next two weeks. If the test runs later than that, move
them or make them again. Put nothing private into them.

| # | Signed in as | Calendar | Event | What gate `CD-G2b` uses it for |
|---|---|---|---|---|
| 1 | A | A's main calendar | `G2b existing event`: one hour, on one day, no guests | An event Fabric did not create: it can be edited only after you approve, and never deleted. |
| 2 | B | B's main calendar | `G2b invitation`: one hour, on one day, guest: A's address | An invitation from another account: A answers it through Fabric. |
| 3 | B | B's main calendar | `G2b weekly invitation`: one hour, repeats weekly, guest: A's address | A recurring invitation: A answers one occurrence only. |
| 4 | A | A's main calendar | `G2b all-day`: the "All day" box ticked, one day | An all-day event: read with its date, never changed. |
| 5 | A | A's main calendar | `G2b weekly`: one hour, repeats weekly, no guests | A recurring event: read as single occurrences, and a change is refused. |

For events 2 and 3, save with "Send" when Google asks whether to notify the guest, so that A really receives an
invitation.

---------------------------------------------------------------------------------------------------------------------

## 9. What was checked against Google's documentation

Checked on 2026-11-17 by reading the pages named below. "Confirmed" means the page states it. Nothing here was
tried in a console: no session may sign in to Google.

| Step | Statement | Status | Google's page |
|---|---|---|---|
| 1.2 | 2-Step Verification is required to sign in to the Cloud console | Confirmed that the requirement exists. Not confirmed: the date it is enforced for a given account | `docs.cloud.google.com/docs/authentication/mfa-requirement` |
| 2.1, 2.2 | Menu > IAM & Admin > Create a Project; Project Name; Location; Create; the project ID cannot be changed | Confirmed. Not stated: what Location shows for a personal account | `developers.google.com/workspace/guides/create-project` |
| 2.3 | The Calendar API asks for no billing account | **Not confirmed.** Google's pages say only "If prompted, enable billing" and that billing "might" be needed for some APIs | the same page; `developers.google.com/identity/protocols/oauth2/native-app` |
| 3.1 to 3.3 | Menu > APIs & Services > Library > Google Workspace; click the API; Enable | Confirmed | `developers.google.com/workspace/guides/enable-apis` |
| 4.1 to 4.5 | Branding page, Get Started, App name, User support email, Audience External, Contact Information, the policy tick box, Create | Confirmed | `developers.google.com/workspace/guides/configure-oauth-consent` |
| 4.3 | Internal is limited to one organization | Confirmed | `support.google.com/cloud/answer/15549945` |
| 4.6 | A new External app starts with publishing status Testing | **Not confirmed** in the text read; the step tells you to look. The button "Publish app" is confirmed | the same page |
| 4.7 | Audience > Test users > Add users > Save; at most 100 test users; a user counts once added | Confirmed. Not stated: whether removing a test user frees its place | the two pages above |
| 5.1 | Data Access > Add or Remove Scopes > Save | Confirmed | `developers.google.com/workspace/guides/configure-oauth-consent` |
| 5.1 | How the dialog lets you enter a scope by its address | **Not confirmed**; the page does not describe the dialog | |
| 5.2 | The five scope names exist and mean what the table says | Confirmed for the three calendar scopes, `openid` and `email` | `developers.google.com/workspace/calendar/api/auth`; `developers.google.com/identity/openid-connect/openid-connect` |
| 5.2 | Which of the scopes Google classes as sensitive | **Not confirmed.** Google's verification page gives "reading events stored in Google Calendar" as an example of a sensitive scope and names no scope. The Data Access page should show the class of each | `developers.google.com/identity/protocols/oauth2/production-readiness/sensitive-scope-verification` |
| 5.2 | Each Calendar call Fabric makes accepts one of the five scopes | Confirmed from each method's page: calendar list accepts `calendar.calendarlist.readonly`; free/busy accepts `calendar.freebusy`; events get, list, insert, update, patch and delete accept `calendar.events` | `developers.google.com/workspace/calendar/api/v3/reference/` |
| 5.4 | A person can untick single scopes on the consent screen, and the app must check what was granted | Confirmed | `developers.google.com/identity/protocols/oauth2/native-app` |
| 6.1, 6.2 | Menu > Google Auth platform > Clients; Create Client; Application type > Desktop app; Name; Create; no further information is needed for a desktop client | Confirmed | `developers.google.com/workspace/guides/create-credentials`; `support.google.com/cloud/answer/15549257` |
| 6.3 | The switch "Use this client for an AI-powered agent", that it cannot be changed after creation, and that Google's hosted MCP servers require it | Confirmed. Not stated by Google: what else the switch changes, if anything | `developers.google.com/workspace/guides/create-credentials`; `developers.google.com/workspace/guides/configure-mcp-servers` |
| 6.4 | The full secret can be viewed and downloaded only once, at creation; later only the last four characters; Add Secret, Disable | Confirmed. **Not confirmed:** the label of the download button and the file's name | `support.google.com/cloud/answer/15549257` |
| 6.6 | A desktop client is a public client that cannot keep a secret | Confirmed | `developers.google.com/identity/protocols/oauth2/native-app` |
| 6.7 | A client unused for six months is deleted, with an email 30 days before | Confirmed | `support.google.com/cloud/answer/15549257` |
| 7 | Seven days for a test user's authorization and its refresh token; the exception for `openid`, email and profile only | Confirmed on two pages | `support.google.com/cloud/answer/15549945`; `developers.google.com/identity/protocols/oauth2` |
| 7 | The warning for test users; the cap of 100 accounts for an unverified app in production, which cannot be reset | Confirmed | `support.google.com/cloud/answer/15549945` |
| 7 | Six months unused; access removed by the user; the limit of 100 refresh tokens per account and client | Confirmed | `developers.google.com/identity/protocols/oauth2` |
| 8 | A new calendar: Add other calendars > Create new calendar > Create calendar; only from a browser | Confirmed | `support.google.com/calendar/answer/37095` |
| 8.1.2, 8.1.3 | Settings and sharing; Shared with; Add people and groups; the five permission names; Send; the other person must click the emailed link | Confirmed | `support.google.com/calendar/answer/37082` |
| 8.2 | How to make an all-day or weekly event and add a guest in the Calendar page | **Not checked**; these are ordinary uses of Google Calendar | |
| 1.3 | What Google asks for when a new Gmail account is created (a phone number, a waiting period) | **Not checked** | |

---------------------------------------------------------------------------------------------------------------------

## 10. Notes for the design, found while checking

These do not change what you do. They are recorded here because they came out of the same reading, and the CD-W05
design takes them up.

- Google now has a fifth access role on a shared calendar, `writerWithoutPrivateAccess` ("Make changes (see private
  events as free/busy)"). The contract `fabric-desktop/1` knows four. The design proposes how to treat it.
- Free/busy needs its own scope: the free/busy call does not accept `calendar.events`.
- Google's pages mark the client secret as optional at the token endpoint and say PKCE should always be used.
  Whether Google accepts a Desktop app client without its secret is not stated; the live test settles it.
- Google's page on conditional changes names update and delete for the `If-Match` header and says inserts have no
  conditional form. It does not name patch. The live test settles that too.
- Google now offers its own hosted Calendar MCP server. Fabric Calendar does not use it: it would put Google, not
  the broker, between the agent and the calendar.

## 11. When you are done

Tell the session three things, and nothing more:

1. That sections 1 to 8 are done, and any label that differed from this document.
2. The path of the client file.
3. Your choice at step 6.3.

The test accounts' addresses, the project ID and the client ID are not secrets, but they do not need to be in the
repository or in a chat.
