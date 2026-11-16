# Fabric Calendar, the desktop application

Work package CD-W02 of the [Calendar Desktop plan](../../docs/plans/calendar-desktop-plan.md): the Tauri shell, the round trip to a Rust broker child, and a packaging spike. Its gate is CD-G1b.

This is a shell with a stand-in broker. The broker of this work package ([`apps/fabric-broker`](../fabric-broker)) has no store, no policy gate, no prover, no Google access and no agent channel, and its requests are made up. The page says so at the top.

## The three parts

```
webview (static HTML, CSS, a small TypeScript layer)
   |  three Tauri commands, one channel
Tauri host (Rust, this directory's src-tauri)
   |  the broker's stdin and stdout, private pipes, fabric-desktop/1 frames
broker child (Rust, apps/fabric-broker)
```

- The webview reaches only the host, and only through `owner_attach`, `owner_send` and `broker_restart`. The capability file [`src-tauri/capabilities/owner-interface.json`](src-tauri/capabilities/owner-interface.json) allows these three and no command of Tauri itself (no window, event, path, shell or file command).
- The host starts one broker with an empty environment (the allowlist in [`relay/src/config.rs`](relay/src/config.rs)), validates every frame in both directions with `fabric-desktop-contracts`, and relays. It builds no frame, answers nothing for the broker, holds no credential and no database, and makes no policy decision.
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

To see the page in a browser with the real broker behind it:

```sh
sh scripts/run.sh cargo build -p fabric-broker
cd apps/fabric-desktop && npm ci && npm run build
npm run harness -- --broker ../../target/debug/fabric-broker
```

With `FABRIC_DESKTOP_TRACE=1` the host writes one line per relayed frame to stderr: the method, the id, the sequence and the size, never the content.

## Identifiers

The owner's prefix is `ai.paracels.fabric`.

| What | Identifier | State |
|---|---|---|
| The application bundle | `ai.paracels.fabric.calendar` | In `src-tauri/tauri.conf.json` |
| The broker executable | `ai.paracels.fabric.calendar.broker` | Proposed for CD-W11; today the bundler signs it under its file name |
| The MCP bridge | `ai.paracels.fabric.calendar.bridge` | Proposed for CD-W06 and CD-W11 |

The host keeps one file outside the bundle: `~/Library/Application Support/ai.paracels.fabric.calendar/instance.lock`, the single-instance lock.

## What a person has to check

`scripts/verify-desktop.sh --packaged` cannot press a key in the application's window, so these checks need a person at the Mac. They are part of gate CD-G1b and are not done until the owner has run them.

Prepare:

```sh
sh scripts/build-desktop.sh --configuration debug
mkdir -p "$HOME/Desktop/Fabric test folder"
rm -rf "$HOME/Desktop/Fabric test folder/Fabric Calendar.app"
cp -R "target/debug/bundle/macos/Fabric Calendar.app" "$HOME/Desktop/Fabric test folder/"
open "$HOME/Desktop/Fabric test folder/Fabric Calendar.app"
```

On macOS the Tab key reaches buttons and radio buttons only when **System Settings, Keyboard, Keyboard navigation** is on. This is the rule for every Mac application, and a person who works by keyboard has it on. Turn it on for part A. Without it, Option+Tab does the same in the webview.

### A. Keyboard only

Do not touch the mouse or the trackpad after the window is in front.

1. Wait about four seconds. The Requests section shows "Create an event, from Synthetic agent (spike)", first as "Received, checking" and then as "Waiting for your decision", with the approval card. Under it a line says how many milliseconds after receipt "Received, checking" was on the page. Expected: at most 500 ms.
2. Press Tab. Focus goes to "Check connection" with a visible outline. Press Space. "Last check" shows a round-trip time. This is the echo.
3. Press Tab. Focus enters the card at "How long this approval lasts", on "Only this request". Press the Down arrow twice and the Up arrow once. The choice moves; nothing is sent.
4. Press Tab twice. Focus is on "Deny", then on "Allow". Press Escape. Expected: the state still says "Waiting for your decision".
5. With focus on "Allow", press Return. Expected: the card closes with "You allowed this. It lasts: until this agent's current session ends." (or the duration you chose), focus is on the card's heading, the state changes to "Fabric checks it once more before it is done" and then to "Not done: Fabric could not prove that it is allowed". The last one is correct here: the stand-in broker has no prover.
6. About a second later a second request arrives: "Change who is invited to an event". When its card is there, Tab to "Deny" and press Space. Expected: "You denied this."
7. Shift+Tab back to "Check connection". Every step back shows the focus outline.

### B. The broker dies and is restarted

1. Quit and open the application again. Wait for the first card. Choose "Until I take it back" in it; do not decide.
2. In Terminal: `pkill -KILL -x fabric-broker`.
3. Expected in the window: a banner "The broker stopped." with the detail "killed by signal 9", a "Restart broker" button, and the card's two buttons disabled with the text that the request is read again after the restart. "Until I take it back" is still chosen.
4. Wait ten seconds. Expected: nothing restarts by itself. `pgrep -x fabric-broker` prints nothing.
5. Tab to "Restart broker" and press Return. Expected: the banner goes away. The request is listed as not available for a moment (the stand-in broker has no store, so it makes the same request again after about three seconds), then the card is back with "Until I take it back" still chosen, and "Allow" works.

### C. A second launch and Quit

1. With the application open, run `"$HOME/Desktop/Fabric test folder/Fabric Calendar.app/Contents/MacOS/fabric-desktop"` in Terminal. Expected: a second window with the banner "Fabric Calendar is already open." and no restart button. `pgrep -x fabric-broker` prints one process id. Close the second window.
2. Press Command+Q in the first window. Expected: `pgrep -x fabric-broker` prints nothing within two seconds.

### D. VoiceOver

Turn VoiceOver on with Command+F5. Open the application as in "Prepare".

1. Press VO+U (VO is Control+Option), choose Headings. Expected: "Fabric Calendar" at level 1; "Broker", "Requests" and "Activity" at level 2; one level 3 heading per request, named after what the agent wants and who asks.
2. Do nothing for four seconds after opening. Expected: VoiceOver says "A request needs your decision: Create an event, from Synthetic agent (spike)." without the VoiceOver cursor moving.
3. Move to the request's heading and read on with VO+Right arrow. Expected order: the state; the line about milliseconds; then the card: Who asks; Where its data goes; In the agent's own words (not checked by Fabric); What it wants to do; Account; Calendar; Title; Starts; Ends; Place; Description; Who is notified; the group "How long this approval lasts" with three radio buttons and which one is selected; Why you are asked, with the rule; This card expires; "Deny, button"; "Allow, button". Each value is read after its term.
4. On "Allow" press VO+Space. Expected: VoiceOver says "You allowed this. …" and the cursor is on the card's heading, not lost at the top of the window.
5. Run `pkill -KILL -x fabric-broker` in Terminal and return to the window. Expected: VoiceOver says "The broker stopped. Nothing can be decided until you restart it." Move to "Deny" in an open card: VoiceOver says it is dimmed and reads the text beside it that explains why.
6. Move to "Restart broker" and press VO+Space. Expected: "The broker is running." and, when the card is usable again, "You can decide again: …".
7. Press Escape with the VoiceOver cursor on "Allow" of an open card. Expected: "Nothing was decided. The request is still waiting for you."

Write down what VoiceOver said where it differs from these lines, and the macOS version.
