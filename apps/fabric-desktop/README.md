# Fabric Calendar, the desktop application

Work packages CD-W02 and CD-W04 of the [Calendar Desktop plan](../../docs/plans/calendar-desktop-plan.md): the Tauri shell and the round trip to a Rust broker child (CD-W02, gate CD-G1b), and the real broker behind it: the encrypted store, the authority core and the owner channel (CD-W04, gate CD-G2a).

The broker ([`apps/fabric-broker`](../fabric-broker)) keeps its state in an encrypted store and survives a restart. It has no rule prover yet (the Isabelle wiring follows CD-W03), no Google access (CD-W05) and no agent channel (CD-W06), so nothing an agent asks is ever carried out: an approved request ends "undecided". A test build can run a synthetic agent so that a person sees requests; it runs only when asked for from a terminal, with a scratch data directory and the test key (see "The synthetic demonstration"). The page says so at the top.

## The three parts

```
webview (static HTML, CSS, a small TypeScript layer)
   |  three Tauri commands, one channel
Tauri host (Rust, this directory's src-tauri)
   |  the broker's stdin and stdout, private pipes, fabric-desktop/1 frames
broker child (Rust, apps/fabric-broker)
```

- The webview reaches only the host, and only through `owner_attach`, `owner_send` and `broker_restart`. The capability file [`src-tauri/capabilities/owner-interface.json`](src-tauri/capabilities/owner-interface.json) allows these three and no command of Tauri itself (no window, event, path, shell or file command).
- The host starts one broker with an empty environment except the broker's three variables (the allowlist in [`relay/src/config.rs`](relay/src/config.rs)): its data directory, where its store key comes from, and, in a test run only, the synthetic agent. It validates every frame in both directions with `fabric-desktop-contracts`, and relays. It builds no frame, answers nothing for the broker, holds no credential, never opens the store, and makes no policy decision.
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

## Build and check

```sh
sh scripts/verify-desktop.sh                      # typecheck, tests, host lint and tests
sh scripts/build-desktop.sh --configuration debug # target/debug/bundle/macos/Fabric Calendar.app
sh scripts/verify-desktop.sh --packaged           # the bundle, from a path with spaces
```

The bundle is arm64, for macOS 15.0 or later, and signed ad hoc. Developer ID signing, notarization and the DMG are CD-W11.

`verify-desktop.sh` also compiles the release broker, which has neither the test key nor the synthetic agent; it does not run it.

To see the page in a browser with the real broker behind it:

```sh
sh scripts/run.sh cargo build -p fabric-broker
cd apps/fabric-desktop && npm ci && npm run build
npm run harness -- --broker ../../target/debug/fabric-broker
```

The harness always starts the broker with the test key and the synthetic agent, in a new temporary data directory that it removes when it stops. It never opens the real store.

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

`FABRIC_DESKTOP_DATA_DIR` moves the single-instance lock and the store to that scratch directory; the store opens with the public test key, so it never touches the Keychain. Once the window has subscribed, "Synthetic agent (test build)" and a synthetic account appear, and about a second later the agent proposes an event; the request waits for your decision. Its classifier is a stand-in that always asks you and never permits, so an approval creates a grant and the request ends "undecided". The agent proposes once more after the first request ends, two requests per run. Quit and start again with the same directory (write it down instead of `$(mktemp -d)`): pending requests, until-revoked grants and decisions are still there.

The host refuses the test key or the synthetic agent without `FABRIC_DESKTOP_DATA_DIR`, and the synthetic agent without the test key: the window then shows a configuration error and no broker starts. A release build ignores all three variables and refuses the test key and the synthetic agent.

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

`scripts/verify-desktop.sh --packaged` cannot press a key in the application's window, so these checks need a person at the Mac. They are part of gate CD-G1b and are not done until the owner has run them.

Prepare. Parts A, B and D need requests, so they run the synthetic demonstration in a scratch directory; part C also checks the real store.

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

On macOS the Tab key reaches buttons and radio buttons only when **System Settings, Keyboard, Keyboard navigation** is on. This is the rule for every Mac application, and a person who works by keyboard has it on. Turn it on for part A. Without it, Option+Tab does the same in the webview.

### A. Keyboard only

Do not touch the mouse or the trackpad after the window is in front.

1. Wait about two seconds. The Requests section shows "Create an event, from Synthetic agent (test build)" as "Waiting for your decision", with the approval card. The synthetic classifier answers at once, so "Received, checking" may be on the page only briefly. Under it a line says how many milliseconds after receipt the request was on the page. Expected: at most 500 ms.
2. Press Tab. Focus goes to "Check connection" with a visible outline. Press Space. "Last check" shows a round-trip time. This is the echo.
3. Press Tab. Focus enters the card at "How long this approval lasts", on "Only this request". Press the Down arrow twice and the Up arrow once. The choice moves; nothing is sent.
4. Press Tab twice. Focus is on "Deny", then on "Allow". Press Escape. Expected: the state still says "Waiting for your decision".
5. With focus on "Allow", press Return. Expected: the card closes with "You allowed this. It lasts: until this agent's current session ends." (or the duration you chose), focus is on the card's heading, the state changes to "Fabric checks it once more before it is done" and then to "Not done: Fabric could not prove that it is allowed". The last one is correct here: this build runs no rule prover.
6. One or two seconds later a second request arrives, another "Create an event". When its card is there, Tab to "Deny" and press Space. Expected: "You denied this."
7. Shift+Tab back to "Check connection". Every step back shows the focus outline.

### B. The broker dies and is restarted

1. Quit, run `SCRATCH="$(mktemp -d)"` for a fresh directory, and open the application again. Wait for the first card. Choose "Until I take it back" in it; do not decide.
2. In Terminal: `pkill -KILL -x fabric-broker`.
3. Expected in the window: a banner "The broker stopped." with the detail "killed by signal 9", a "Restart broker" button, and the card's two buttons disabled with the text that the request is read again after the restart. "Until I take it back" is still chosen.
4. Wait ten seconds. Expected: nothing restarts by itself. `pgrep -x fabric-broker` prints nothing.
5. Tab to "Restart broker" and press Return. Expected: the banner goes away. The same request is back from the store, still waiting for your decision; the card is usable again after the page reads it once more, with "Until I take it back" still chosen, and "Allow" works. The synthetic agent of the new run proposes its own requests after that; the restarted broker does not make the old request again.
6. Quit and open the application again with the same `$SCRATCH`. Expected: the decision of step 5 is still there (the request is no longer listed as waiting), and no request is listed twice.

### C. A second launch and Quit

1. With the application open, open a second Terminal window. `fabric_demo` is defined only in the first window, so give the whole command there, with the directory that the first window printed:

   ```sh
   SCRATCH=<the same directory>
   FABRIC_DESKTOP_DATA_DIR="$SCRATCH" FABRIC_BROKER_KEY_SOURCE=test FABRIC_BROKER_SYNTHETIC_AGENT=1 \
     "$HOME/Desktop/Fabric test folder/Fabric Calendar.app/Contents/MacOS/fabric-desktop"
   ```

   Expected: a second window with the banner "Fabric Calendar is already open." and no restart button. `pgrep -x fabric-broker` prints one process id. Close the second window.
2. Press Command+Q in the first window. Expected: `pgrep -x fabric-broker` prints nothing within two seconds.
3. Open the copied application from Finder, without the variables: this is the product's store. Expected: on the first launch macOS asks whether Fabric Calendar may use its Keychain item; allow it. The page shows the store as ready, the prover as unavailable, and no agent, account or request. `ls -l "$HOME/Library/Application Support/ai.paracels.fabric.calendar/broker"` lists `broker.db` and `broker.lock`, owner-only. Quit.

### D. VoiceOver

Turn VoiceOver on with Command+F5. Open the application as in "Prepare" (`fabric_demo`, with a fresh `$SCRATCH`).

1. Press VO+U (VO is Control+Option), choose Headings. Expected: "Fabric Calendar" at level 1; "Broker", "Requests" and "Activity" at level 2; one level 3 heading per request, named after what the agent wants and who asks.
2. Do nothing for four seconds after opening. Expected: VoiceOver says "A request needs your decision: Create an event, from Synthetic agent (test build)." without the VoiceOver cursor moving.
3. Move to the request's heading and read on with VO+Right arrow. Expected order: the state; the line about milliseconds; then the card: Who asks; Where its data goes; In the agent's own words (not checked by Fabric); What it wants to do; Account; Calendar; Title; Starts; Ends; Place; Description; Who is notified; the group "How long this approval lasts" with three radio buttons and which one is selected; Why you are asked, with the rule; This card expires; "Deny, button"; "Allow, button". Each value is read after its term.
4. On "Allow" press VO+Space. Expected: VoiceOver says "You allowed this. …" and the cursor is on the card's heading, not lost at the top of the window.
5. Run `pkill -KILL -x fabric-broker` in Terminal and return to the window. Expected: VoiceOver says "The broker stopped. Nothing can be decided until you restart it." Move to "Deny" in an open card: VoiceOver says it is dimmed and reads the text beside it that explains why.
6. Move to "Restart broker" and press VO+Space. Expected: "The broker is running." and, when the card is usable again, "You can decide again: …".
7. Press Escape with the VoiceOver cursor on "Allow" of an open card. Expected: "Nothing was decided. The request is still waiting for you."

Write down what VoiceOver said where it differs from these lines, and the macOS version.
