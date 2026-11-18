# Fabric Calendar, the desktop application

Work packages CD-W02, CD-W04 and CD-W08 of the [Calendar Desktop plan](../../docs/plans/calendar-desktop-plan.md): the Tauri shell and the round trip to a Rust broker child (CD-W02, gate CD-G1b), the real broker behind it: the encrypted store, the authority core and the owner channel (CD-W04, gate CD-G2a), and the screens of the owner's interface (CD-W08, gate CD-G3a).

The broker ([`apps/fabric-broker`](../fabric-broker)) keeps its state in an encrypted store and survives a restart. It has no Google access (CD-W05) and no agent channel (CD-W06), and the application does not ship Isabelle yet, so in the product nothing an agent asks is carried out: an approved request ends "undecided". A test build can run a synthetic agent so that a person sees requests; it runs only when asked for from a terminal, with a scratch data directory and the test key (see "The synthetic demonstration"). A test build given the paths of an Isabelle installed in the checkout also proves every request with the real rule prover and carries the approved ones out on a practice calendar held in memory (see "The practice run"). The strip at the top of the page says when the rule checker is not running, and a second line under the title (the practice line) says that a request is carried out only on practice data and that nothing changes a real calendar.

## The three parts

```
webview (static HTML, CSS, a small TypeScript layer)
   |  three Tauri commands, one channel
Tauri host (Rust, this directory's src-tauri)
   |  the broker's stdin and stdout, private pipes, fabric-desktop/1 frames
broker child (Rust, apps/fabric-broker)
```

- The webview reaches only the host, and only through `owner_attach`, `owner_send` and `broker_restart`. The capability file [`src-tauri/capabilities/owner-interface.json`](src-tauri/capabilities/owner-interface.json) allows these three and no command of Tauri itself (no window, event, path, shell or file command).
- The host starts one broker with an empty environment except the broker's variables (the allowlist in [`relay/src/config.rs`](relay/src/config.rs)): its data directory and where its store key comes from; the synthetic agent only in a test run with a scratch directory and the test key; and, in a debug build, the three paths of the rule prover (Isabelle, Isabelle's per-user directory and the rule theory) when all three are set, with any store. With the real store nothing is carried out, so an approved request ends "undecided". On Unix it adds one fixed `PATH`, `/usr/bin:/bin:/usr/sbin:/sbin`, never its own, because Isabelle's scripts call system tools such as `/usr/sbin/sysctl` by name. It validates every frame in both directions with `fabric-desktop-contracts`, and relays. It builds no frame, answers nothing for the broker, holds no credential, never opens the store, and makes no policy decision.
- The contract is [`contracts/desktop/v1`](../../contracts/desktop/v1/README.md). The webview imports its TypeScript types; it does not copy them.

| Path | What it holds |
|---|---|
| `ui/` | `index.html`, `styles.css` and `src/*.ts`: the page |
| `test/` | Tests of the page's logic, run by Node without a browser |
| `harness/` | A development server that shows the same page in an ordinary browser, with the real broker behind it. It is not shipped. |
| `build.ts` | The frontend build: Node strips the types; nothing is bundled |
| `relay/` | The crate `fabric-desktop-relay`: the host's logic without Tauri. A member of the root Cargo workspace, so every CI runner tests it. |
| `src-tauri/` | The Tauri host. Its own Cargo workspace, because Tauri builds only where its platform libraries are. |

npm packages, all for development and pinned: `@tauri-apps/cli`, `typescript`, `@types/node`. The page loads no package at run time.

## The screens

The page has five screens (CD-W08) and, above them, a strip that says what needs the owner now: the broker stopped, a store that cannot be opened with the broker's reason, the rule checker not running (with the cause the broker names and what you can do about it), an account that needs a new sign-in, and how many requests wait for a decision.

| Screen | What it does | Owner methods |
|---|---|---|
| Review agent actions | The open requests and those that ended while the window was open, each with its state. The approval card. A blocked request with its failing rule, the broker's explanation and its proof. The details of a request: the decision, the proofs, the authorization, the attempts, the calendar's event. The permissions the owner gave, each with "Take back the permission". The agents, with pause and resume of one or of all. | `decide_request`, `get_request_details`, `get_proof`, `revoke_grant`, `pause_agent` |
| Agent permissions | The profiles (Careful, Flexible and custom ones) and the ten rows of the profile shown. Puts a profile in force. Saves a custom profile; when the save would permit more than before, shows the rows the broker lists and asks the owner to confirm them. | `save_profile` |
| Contacts | Groups of exact addresses and domains: new, change, delete. | `save_group` |
| Calendar preferences | The time zone, working hours, protected times, uninterrupted time, time between events and the calendars used for warnings. They are only stored; the warnings are CD-W09. | `save_preferences` |
| Status | The connection with "Check connection", the store, the rule checker, the accounts and their calendars as the broker reports them, and the activity. | `hello` |

The page decides nothing about permissions and keeps no state of the broker: after `hello`, `get_snapshot` and `subscribe` it shows what the snapshot and the notices carry. Every command carries a command id and the revision it was written against. A form whose stored version changed while the owner was editing keeps the owner's text, shows the stored values beside it, and can be saved only after the owner pressed "I have seen the current version"; a `conflict` answer leads to the same. A command that got no answer is sent again with the same command id.

Not built, because the broker or the contract has no command for it yet: connecting a Google account (`connect_google`, CD-W05), choosing calendars and naming the Agent Drafts calendar, connecting and enrolling an agent and the local task entry (CD-W06, CD-W07), narrowing a request on the approval card, importing from Google contacts, listing requests that ended before the window opened, cancelling a request, resolving an unknown outcome by hand, and deleting a custom profile.

In `ui/src` each screen has a model without the DOM and a view (`review`, `profiles`, `groups`, `preferences`). `model.ts` is the core: the connection, the snapshot, the notices and the commands. `nav.ts`, `status.ts` and `shell.ts` are the frame around the screens, and `text.ts` holds the words about requests. The tests in `test/` drive the models against a fake host and judge every frame, sent or fed, with the contract's own checker.

## Build and check

```sh
sh scripts/verify-desktop.sh                      # typecheck, tests, host lint and tests
sh scripts/build-desktop.sh --configuration debug # target/debug/bundle/macos/Fabric Calendar.app
sh scripts/verify-desktop.sh --packaged           # the bundle, from a path with spaces
sh scripts/verify-desktop.sh --live-proofs        # the broker with the real Isabelle, never in CI
```

`--live-proofs` runs the tests that need the real rule prover: the broker's live tests ([`apps/fabric-broker/tests/live.rs`](../fabric-broker/tests/live.rs)), the first of them alone and by name, and the rule theory's corpus. Install Isabelle first, by hand, with `sh scripts/setup-demo.sh`; the script never installs it and stops if it is missing. It takes Isabelle's place from the three variables of "The practice run", all three or none; with none, from the checkout (`.tooling/isabelle` and `isabelle`). Every test uses the test key in a temporary directory. The first run can take several minutes while Isabelle builds its session heap. The live proofs assert that the in-process stop of the prover is acknowledged within 1.5 s (the prover gives Isabelle at most 1.2 s to stop and then ends it), so run them on an otherwise idle Mac.

The bundle is arm64, for macOS 15.0 or later, and signed ad hoc. Developer ID signing, notarization and the DMG are CD-W11.

`verify-desktop.sh` also compiles the release broker, which has neither the test key nor the synthetic agent; it does not run it.

To see the page in a browser with the real broker behind it:

```sh
sh scripts/run.sh cargo build -p fabric-broker
cd apps/fabric-desktop && npm ci && npm run build
npm run harness -- --broker ../../target/debug/fabric-broker
```

The harness always starts the broker with the test key and the synthetic agent, in a new temporary data directory that it removes when it stops. It never opens the real store. Started with the three variables of "The practice run" set, it passes them to the broker, which then proves requests and uses the practice calendar; with one or two of them set it starts nothing. On Unix it gives the broker the host's fixed `PATH` ("The three parts").

The harness has switches for states that are hard to reach by hand:

```sh
curl -X POST http://127.0.0.1:4317/harness/kill-broker     # the broker dies; the page shows "The broker stopped."
curl -X POST http://127.0.0.1:4317/harness/hold-store      # while no broker runs: another process holds the store's lock
curl -X POST http://127.0.0.1:4317/harness/release-store   # that process ends
npm run harness -- --broker harness/replay-broker.ts --port 4319
```

With the store held, "Restart broker" in the page starts a broker that cannot open its store, and the strip shows the broker's reason and "Read the current state again"; after the release the broker opens the store at the next request that comes five seconds or more after its last try. The last line runs a stand-in in place of the broker. It replays the contract's fixtures and changes nothing, to look at what this broker cannot show yet, or shows only in the practice run: a blocked request with its proof, a request that was carried out, a failure, requests that no proof decided (each with the reason the broker recorded), several agents, an account that needs a new sign-in, the causes of a rule checker that is not running, one after the other, and a card that offers three durations (made in the stand-in, since no fixture proposes more than one request). It is for looking at the page and never evidence for a gate.

With `FABRIC_DESKTOP_TRACE=1` the host writes one line per relayed frame to stderr: the method, the id, the sequence and the size, never the content.

## The store

The broker keeps everything it decides in one SQLite database encrypted with SQLCipher (Apple's CommonCrypto, a raw 256-bit key): requests, approvals, grants, sessions, the command table, a hash-chained ledger of every change, and an access audit. One broker writes it at a time (`broker.lock`); a second one answers `store_unavailable`. A killed broker loses nothing it had answered for: the next start checks the file, the ledger and the invariants, ends every session of the previous run and its session grants (`broker_restart`), voids unused authorizations, and keeps pending requests, until-revoked grants, approvals and the command table, so deciding the same way again after a restart gets the same answer.

| What | Where |
|---|---|
| The store | `~/Library/Application Support/ai.paracels.fabric.calendar/broker/broker.db` (with `-wal` and `-shm`), the directory mode 0700 and the files 0600 |
| Its lock | `.../broker/broker.lock`; the backups taken before a migration: `.../broker/backups/` |
| The store key | The login Keychain, a generic password: service `ai.paracels.fabric.calendar.store`, account `database-key-v1` |

The first launch creates the key and the store. macOS asks once whether Fabric Calendar may use the Keychain item, and again after every rebuild, because an ad-hoc signed build is a new code identity. If the Keychain refuses, the page shows the store as refused and nothing is written in plain text.

A one-request grant stays live until its request is terminal, also while the request is `applied` or `outcome_unknown`: retries of the same change need it, and no other request can use it. Nothing reaches those states before CD-W05.

The bundle will ship SQLCipher (BSD-3-Clause) and rusqlite (MIT); their notices go into the bundle with CD-W11.

## The synthetic demonstration

A debug build can show requests before an agent can connect. Start it from Terminal, never against the real store:

```sh
sh scripts/build-desktop.sh --configuration debug
FABRIC_DESKTOP_DATA_DIR="$(mktemp -d)" FABRIC_BROKER_KEY_SOURCE=test FABRIC_BROKER_SYNTHETIC_AGENT=1 \
  "target/debug/bundle/macos/Fabric Calendar.app/Contents/MacOS/fabric-desktop"
```

`FABRIC_DESKTOP_DATA_DIR` moves the single-instance lock and the store to that scratch directory; the store opens with the public test key, so it never touches the Keychain. Once the window has subscribed, "Synthetic agent (test build)" and a synthetic account appear, and about a second later the agent proposes an event; the request waits for your decision. Without the rule prover's variables ("The practice run") its classifier is a stand-in that always asks you and never permits, so an approval creates a grant and the request ends "undecided". The agent proposes once more after the first request ends, two requests per run. Quit and start again with the same directory (write it down instead of `$(mktemp -d)`): pending requests, until-revoked grants and decisions are still there.

The host refuses the test key or the synthetic agent without `FABRIC_DESKTOP_DATA_DIR`, and the synthetic agent without the test key: the window then shows a configuration error and no broker starts. A release build ignores all of these variables, the rule prover's three too, and refuses the test key and the synthetic agent.

## The practice run

A debug build can show the whole path of a request: the agent asks, you approve, the real rule prover (Isabelle) proves it, Fabric authorizes it once, carries it out and records it. It is carried out on a practice calendar that exists only in the broker's memory, in debug builds, with the synthetic agent and the test key. **Everything in this run is practice data.** No real calendar changes, nothing reaches Google, and the practice calendar is gone when the broker stops.

Prepare once, by hand: `sh scripts/setup-demo.sh` installs Isabelle in `.tooling/isabelle` of the checkout. Then build the debug application and start it from the checkout's root, with a new scratch directory each time:

```sh
sh scripts/build-desktop.sh --configuration debug
D="$(mktemp -d)"
FABRIC_DESKTOP_DATA_DIR="$D" FABRIC_BROKER_KEY_SOURCE=test FABRIC_BROKER_SYNTHETIC_AGENT=1 \
FABRIC_BROKER_ISABELLE_HOME="$PWD/.tooling/isabelle/Isabelle2025-2" \
FABRIC_BROKER_ISABELLE_USER_HOME="$PWD/.tooling/isabelle/home" \
FABRIC_BROKER_RULES_DIR="$PWD/isabelle" \
  "target/debug/bundle/macos/Fabric Calendar.app/Contents/MacOS/fabric-desktop"
```

The three `FABRIC_BROKER_ISABELLE_HOME`, `FABRIC_BROKER_ISABELLE_USER_HOME` and `FABRIC_BROKER_RULES_DIR` go together, as absolute paths: with one or two of them the host starts no broker. The broker never looks for Isabelle by itself. The host gives the broker no `PATH` of its own: on Unix the broker gets the fixed `/usr/bin:/bin:/usr/sbin:/sbin`, where Isabelle's scripts find the system tools they call ("The three parts"). The same run in a browser, once "Build and check" has built the broker and the page: from the checkout's root, keep the root in a variable before `cd`, because a `$PWD` written after it would point into `apps/fabric-desktop` and the broker would refuse the paths.

```sh
R="$PWD"
cd apps/fabric-desktop
FABRIC_BROKER_ISABELLE_HOME="$R/.tooling/isabelle/Isabelle2025-2" \
FABRIC_BROKER_ISABELLE_USER_HOME="$R/.tooling/isabelle/home" \
FABRIC_BROKER_RULES_DIR="$R/isabelle" \
  npm run harness -- --broker ../../target/debug/fabric-broker
```

The words "the calendar" in the request states mean the practice calendar in this run, as the practice line under the page's title says. What you see:

1. The build line under the title reads "Development build. Broker: fabric-broker, build 0.1.0 CD-W04 synthetic practice."; below it the practice line says that a request is carried out only on practice data and that nothing changes a real calendar. The strip says "The rule checker is starting." with "Requests wait until it is ready." The first start can take several minutes while Isabelle builds its session heap; later starts take seconds.
2. That entry of the strip goes away. The Status screen says "Running." under "Rule checker", and its activity has "Rule checker: Running. Store: Open". Under "Agents", "Synthetic agent (test build)" is connected. The agent proposes only once the rule checker is ready.
3. Request A, "Create an event, from Synthetic agent (test build)": "Received, checking", then "Waiting for your decision", with the approval card. "Why you are asked" names "Rule: authority", and "Show the proof" opens the Isabelle proof of that ruling. Choose "Only this request" and Allow.
4. Request A goes on: "You allowed it; Fabric checks it once more before it is done", "Fabric checks it once more before it is done", "Allowed; waiting to be sent to the calendar", "Being sent to the calendar", then "Done: the calendar accepted it. Fabric has not confirmed it by reading it back yet". Some of these pass quickly; the activity on the Status screen lists each. The event was made on the practice calendar. No confirmation comes in this build, so the request stays there and counts as in progress. Its details list the decision, both proofs (the ruling and the check before carrying it out), the authorization and the attempt.
5. Request B, "Delete an event", of an event on the practice calendar that Fabric did not make: "Blocked by a rule", with "Rule fabric_created: Only events that Fabric created may be deleted." and the proof's id. Nothing asks you; the rule prover blocked it. "Show the proof" shows the proof: outcome "Blocked", failing rule `fabric_created`.
6. Request C, "Change an event", moves A's event by 30 minutes and notifies nobody: a card. Allow once; it ends "Done: the calendar accepted it. Fabric has not confirmed it by reading it back yet".
7. Request D, "Delete an event", deletes A's event and notifies nobody: a card (a delete always asks). Allow once; it ends the same way. The agent's script is finished.

If request A ends any other way, C and D are skipped and B still runs. When something fails, nothing is carried out:

- Without the three variables the synthetic agent works as in "The synthetic demonstration": a card, then "Not done: Fabric could not prove that it is allowed".
- One or two of the variables, or a path that is not absolute: the strip says "Fabric Calendar is not installed correctly." with the host's reason under "Details", and no broker starts.
- A path that holds no Isabelle or no rule theory stops the broker at start: the strip says "The broker stopped." with its exit code under "Details", and the Terminal shows the broker's one line naming the variable and the path.
- A rule theory that is not the one the broker was built with leaves the rule checker not running for the run: the strip says "The rule checker is not running."
- If Isabelle stops, or does not answer within a check's limit and a few seconds more, the request ends "Not done: Fabric could not prove that it is allowed". The strip says the rule checker is not running, then that it is starting, and the entry goes away when it is ready again.
- A check that reaches its limit (30 s, or less for a request that waited behind other checks: its time ends at the request's deadline) ends the request the same way, and the rule checker keeps running. With the full 30 s the request has an undecided proof and the strip shows nothing. A request that waited ends at its deadline without a proof, since Isabelle's answer comes after it, and the strip may say for a moment, until Isabelle answers, that the rule checker is not running.

Use a new scratch directory for every practice run. A restart of the broker starts a fresh practice calendar, while the store keeps the requests that were done: on a reused directory those requests show done for events the practice calendar no longer has. Requests that are done stay open until CD-W05 can confirm them, and each agent may have 25 open requests, so about eight runs fit in one directory before the agent is told it is busy.

## Identifiers

The owner's prefix is `ai.paracels.fabric`.

| What | Identifier | State |
|---|---|---|
| The application bundle | `ai.paracels.fabric.calendar` | In `src-tauri/tauri.conf.json` |
| The broker executable | `ai.paracels.fabric.calendar.broker` | Proposed for CD-W11; today the bundler signs it under its file name |
| The MCP bridge | `ai.paracels.fabric.calendar.bridge` | Proposed for CD-W06 and CD-W11 |
| The store key in the Keychain | service `ai.paracels.fabric.calendar.store`, account `database-key-v1` | Proposed in CD-W04; the names are open for the owner to confirm |

Outside the bundle the host keeps `~/Library/Application Support/ai.paracels.fabric.calendar/instance.lock`, the single-instance lock, and the broker keeps its store in `broker/` beside it ("The store").

## What a person has to check

`scripts/verify-desktop.sh --packaged` cannot press a key in the application's window, so these checks need a person at the Mac. They are part of gates CD-G1b and CD-G3a and are not done until the owner has run them. The steps of parts A, B and E were walked by keyboard in a browser with the same broker behind the page (the harness, "Build and check"); the packaged window uses another web engine, and VoiceOver was not run.

Prepare. Parts A, B, D and E need requests, so they run the synthetic demonstration in a scratch directory; part C also checks the real store.

```sh
sh scripts/build-desktop.sh --configuration debug
mkdir -p "$HOME/Desktop/Fabric test folder"
rm -rf "$HOME/Desktop/Fabric test folder/Fabric Calendar.app"
cp -R "target/debug/bundle/macos/Fabric Calendar.app" "$HOME/Desktop/Fabric test folder/"
SCRATCH="$(mktemp -d)"; echo "$SCRATCH"
fabric_demo() {
  FABRIC_DESKTOP_DATA_DIR="$SCRATCH" FABRIC_BROKER_KEY_SOURCE=test FABRIC_BROKER_SYNTHETIC_AGENT=1 \
    "$HOME/Desktop/Fabric test folder/Fabric Calendar.app/Contents/MacOS/fabric-desktop"
}
fabric_demo
```

"Open the application" below means `fabric_demo` in that Terminal window; quit with Command+Q.

On macOS the Tab key reaches buttons and radio buttons only when **System Settings, Keyboard, Keyboard navigation** is on. This is the rule for every Mac application, and a person who works by keyboard has it on. Turn it on for parts A and E. Without it, Option+Tab does the same in the webview.

### A. Keyboard only: the requests

Do not touch the mouse or the trackpad after the window is in front.

1. Wait about two seconds. The strip at the top says "The rule checker is not running." and "1 request waits for your decision." The screen "Review agent actions" lists "Create an event, from Synthetic agent (test build)" as "Waiting for your decision", with the approval card. The synthetic classifier answers at once, so "Received, checking" may be on the page only briefly. A line under the state says how many milliseconds after receipt the request was on the page. Expected: at most 500 ms.
2. Press Tab six times: the five screens of the navigation, then "Review the waiting requests" in the strip, each with a visible outline. Press Return. Expected: focus is on the request's heading.
3. Read the card. Under "How long this approval lasts" it says "Only this request." and offers no choice: in this run no card offers a longer permission. Press Tab. Expected: focus is on "Show the proof".
4. Press Tab twice. Focus is on "Deny", then on "Allow". Press Escape. Expected: the state still says "Waiting for your decision".
5. With focus on "Allow", press Return. Expected: the card closes with "You allowed this, for this one request.", focus is on the request's heading, and the state changes to "Fabric checks it once more before it is done" and then to "Not done: Fabric could not prove that it is allowed". If the broker recorded why, a line under it says so; in this run it would read "Reason: The rule checker was starting or not running." That state is correct here: this run has no rule prover. Under "Permissions you gave" the permission is listed as "Ended while this window was open.", with "The one request it was given for has ended." in its last line.
6. Press Tab through "Permissions you gave". Expected: that permission has no "Take back the permission" button, because it has ended. Taking a permission back is in part F.
7. One or two seconds after the first request ended a second request arrived, another "Create an event". Press Shift+Tab back to its card, to "Deny", and press Space. Expected: "You denied this."
8. Press Tab until focus is on "Pause Synthetic agent (test build)" under "Agents", and press Return. Expected: the agent's state says "Paused.", and the button now reads "Resume" and still has the focus. Press Return again. Expected: "Connected."
9. Go to "Status" in the navigation and press Return. Expected: only the Status screen is shown and focus is on its heading. Press Tab to "Check connection" and press Space. "Last check" shows a round-trip time. This is the echo.

### B. The broker dies and is restarted

1. Quit, run `SCRATCH="$(mktemp -d)"` for a fresh directory, and open the application again. Wait for the first card; do not decide.
2. In Terminal: `pkill -KILL -x fabric-broker`.
3. Expected in the window: the strip says "The broker stopped." with "Details: killed by signal 9" and a "Restart broker" button, and the card's two buttons are disabled with the text that the request is read again after the restart.
4. Wait ten seconds. Expected: nothing restarts by itself. `pgrep -x fabric-broker` prints nothing.
5. Tab to "Restart broker" and press Return. Expected: that entry of the strip goes away. The same request is back from the store, still waiting for your decision; the card is usable again after the page reads it once more, and "Allow" works. The synthetic agent of the new run proposes its own requests after that; the restarted broker does not make the old request again.
6. Quit and open the application again with the same `$SCRATCH`. Expected: the decision of step 5 is still there (the request is no longer listed as waiting), and no request is listed twice.

### C. A second launch and Quit

1. With the application open, open a second Terminal window. `fabric_demo` is defined only in the first window, so give the whole command there, with the directory that the first window printed:

   ```sh
   SCRATCH=<the same directory>
   FABRIC_DESKTOP_DATA_DIR="$SCRATCH" FABRIC_BROKER_KEY_SOURCE=test FABRIC_BROKER_SYNTHETIC_AGENT=1 \
     "$HOME/Desktop/Fabric test folder/Fabric Calendar.app/Contents/MacOS/fabric-desktop"
   ```

   Expected: a second window whose strip says "Fabric Calendar is already open." and has no restart button. `pgrep -x fabric-broker` prints one process id. Close the second window.
2. Press Command+Q in the first window. Expected: `pgrep -x fabric-broker` prints nothing within two seconds.
3. Open the copied application from Finder, without the variables: this is the product's store. Expected: on the first launch macOS asks whether Fabric Calendar may use its Keychain item; allow it. The strip says "The rule checker is not running." and nothing else; the Status screen says "Running and up to date", the store "Open." and "No account is connected."; no request, permission or agent is listed. `ls -l "$HOME/Library/Application Support/ai.paracels.fabric.calendar/broker"` lists `broker.db` and `broker.lock`, owner-only. Quit.

### D. VoiceOver

Turn VoiceOver on with Command+F5. Open the application as in "Prepare" (`fabric_demo`, with a fresh `$SCRATCH`).

1. Press VO+U (VO is Control+Option), choose Headings. Expected: "Fabric Calendar" at level 1; "Review agent actions" at level 2; "Requests", "Permissions you gave" and "Agents" at level 3; at level 4 one heading per request, named after what the agent wants and who asks, and one per agent. Choose Landmarks. Expected: a banner, the navigation "Screens", the region "Needs your attention", main, and the region "Review agent actions".
2. Do nothing for four seconds after opening. Expected: VoiceOver says "A request needs your decision: Create an event, from Synthetic agent (test build)." without the VoiceOver cursor moving.
3. Move to the request's heading and read on with VO+Right arrow. Expected order: the state; the line about milliseconds; then the card: Who asks; Where its data goes; In the agent's own words (not checked by Fabric); What it wants to do; Account; Calendar; Title; Starts; Ends; Place; Description; Who is notified; the group "How long this approval lasts" with "Only this request." and the sentence that Fabric offers a longer permission only where it would also serve later requests; Why you are asked, with the rule; This card expires; the proof's id; "Show the proof" with the proof's id, a collapsed button; "Deny, button"; "Allow, button"; "Details of Create an event, from Synthetic agent (test build)", a collapsed button. Each value is read after its term.
4. On "Allow" press VO+Space. Expected: VoiceOver says "You allowed this, for this one request." and the cursor is on the request's heading, not lost at the top of the window.
5. Run `pkill -KILL -x fabric-broker` in Terminal and return to the window. Expected: VoiceOver says "The broker stopped. Nothing can be decided until you restart it." Move to "Deny" in an open card: VoiceOver says it is dimmed and reads the text beside it that explains why.
6. Move to "Restart broker" and press VO+Space. Expected: "The broker is running." and, when the card is usable again, "You can decide again: …".
7. Press Escape with the VoiceOver cursor on "Allow" of an open card. Expected: "Nothing was decided. The request is still waiting for you."
8. Open "Agent permissions" from the navigation. Expected: VoiceOver reads the heading "Agent permissions". The group "Profile shown below" has the radio buttons "Careful, built in, in force" and "Flexible, built in". Each of the eight rows is a group named after what it covers, with the radio buttons Allow, Ask and Block and one sentence that says what the chosen one means; "Delete events" and "Delete a calendar or change who it is shared with" have no buttons and say why.
9. Choose "Ask" in the first row, move to "Save as a custom profile" and press VO+Space while the name is empty. Expected: VoiceOver says "Give the profile a name." and the cursor is in the field "Name of the new profile".

Write down what VoiceOver said where it differs from these lines, and the macOS version.

### E. Keyboard only: the other screens

Open the application as in "Prepare", with a fresh `$SCRATCH`. Do not touch the mouse or the trackpad.

1. Open "Agent permissions". Expected: "Careful" is chosen in "Profile shown below" and tagged "built in" and "in force"; "Put Careful in force" is dimmed with "Careful is in force already." beside it.
2. Press the Right arrow in the profile list. Expected: the rows show Flexible. Tab to "Put Flexible in force" and press Return. Expected: Flexible is tagged "in force". Choose Careful again and put it in force.
3. With Careful shown, Tab to the row "Read the details of events" and press the Left arrow: "Allow". Expected: under the row "Now in Careful: Ask"; Careful is tagged "unsaved changes"; a field "Name of the new profile" is there, after the rows. Type a name. Tab to "Save as a custom profile" and press Return. Expected: a box "This profile permits more than before:" with "Read the details of events: from Ask to Allow"; focus is on its heading. Tab to "Confirm and save" and press Return. Expected: the new profile is in the list, chosen, tagged "custom, version 1"; Careful is still "in force".
4. Open "Contacts". Tab to "New group" and press Return. Expected: focus is on the heading "New group". Type a name; in "Addresses, one per line" type `Ana.B@Example.COM` and, on a second line, `not an address`. Expected: "Line 2 is not an address: not an address" under the field, and the same beside the dimmed save button. Remove the second line, Tab to "Save the new group" and press Return. Expected: the group is listed with "1 address and 0 domains." and focus is on its heading.
5. Tab to "Edit the group" of that group and press Return. Expected: the address reads `Ana.B@example.com`: its part before the @ as typed, its domain in lower case. Tab to "Discard changes" and press Return. Tab to "Delete the group" and press Return. Expected: the question "Delete …? Permissions that name it end.", with focus on "Keep it". Tab to "Yes, delete the group" and press Return. Expected: "No group yet."
6. Open "Calendar preferences". Expected on a first run: the time zone is empty, "Set my working hours", "Keep a block of free time" and "Leave time between events" are off, there is no protected time, and "Save preferences" is dimmed with "Nothing has changed." Tab to "Use this computer's zone" and press Return. Tab to "Set my working hours" and press Space. Expected: no day is chosen and both times are empty, and the save button's text names what is missing. Choose days with Space, type the two times, Tab to "Save preferences" and press Return. Quit and open the application again with the same `$SCRATCH`. Expected: the values are still there.
7. A store that cannot be opened. Quit. In a second Terminal window start the broker of the bundle by hand on the same store; it waits for input and holds the store's lock:

   ```sh
   SCRATCH=<the same directory>
   FABRIC_BROKER_DATA_DIR="$SCRATCH/broker" FABRIC_BROKER_KEY_SOURCE=test \
     "$HOME/Desktop/Fabric test folder/Fabric Calendar.app/Contents/MacOS/fabric-broker"
   ```

   Open the application in the first window. Expected: the strip says "The broker cannot open its store." with the broker's reason ("Another Fabric Calendar broker is using the store. …") and a button "Read the current state again"; no screen can save anything, and each save button says why. Press Control+D in the second window, wait five seconds, Tab to "Read the current state again" and press Return. Expected: the entry goes away and the screens show the stored state.

### F. A card with three durations

This part needs the broker's pull request for the duration rule and the practice run. Until that pull request is merged every card offers only "Only this request", and in the synthetic demonstration without the rule checker no card ever offers more. With it, a card offers more only for a "Create an event" that the profile in force allows: in the practice run that is request A, and only if Flexible was in force when its card opened. Its steps were walked in a browser with that pull request's broker behind the page (the harness, "Build and check"), by keyboard but for step 1: the rule checker was ready within seconds, so a script put Flexible in force. The way out at the end of step 1 was walked with the harness's kill switch and "Restart broker" in place of quitting. Not walked in the packaged window.

1. Start the practice run ("The practice run") with a fresh scratch directory and write the directory down. While the strip says "The rule checker is starting.", open "Agent permissions", choose Flexible and put it in force (part E, step 2). Expected: Flexible is tagged "in force" before the first request arrives. The first start leaves minutes for this, a later start only seconds. A card keeps the durations it had when it opened: if request A's card was there before Flexible was in force, it says "Only this request." and stays so. Then quit and start the practice run again with the same directory. Flexible is still in force. A request of the first start that still waits is listed again and its card still says "Only this request."; use the "Create an event" that the new start adds, the last request in the list.
2. Go back to "Review agent actions" and use the card of request A, "Create an event, from Synthetic agent (test build)". Expected: "How long this approval lasts" has three radio buttons, "Only this request", "Until this agent's current session ends" and "Until I take it back", and "Only this request" is selected.
3. Press Tab to the radio buttons and press the Down arrow twice. Expected: "Until I take it back" is selected, and a sentence under the choices says that the permission covers more than the request shown: every new event this agent creates in the calendar shown above, for as long as the permission lasts. Press the Up arrow twice. Expected: "Only this request" is selected and the sentence is gone.
4. Choose "Until I take it back" again, press Tab to "Allow" and press Return. Expected: the card closes with "You allowed this. The permission lasts until you take it back." Under "Permissions you gave" the permission is listed as "In force.", with "Until you take it back. It has no end date." and the profile it was given under.
5. Press Tab until focus is on "Take back the permission" of that permission, and press Return. Expected: the permission stays on the page as "Ended while this window was open.", with "You took it back." in its last line, and focus is on its heading.
