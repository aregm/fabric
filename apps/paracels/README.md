# Paracels

An Expo (React Native) app for iOS, built with Expo SDK 58 and Expo Router. It lives in the Fabric
repository but is not part of the Cargo workspace.

It is a one-screen chat: Alice and Bob in one room, talking to servers on this Mac. The
[chat prototype spec](../../docs/paracels-chat.md) holds the decisions, the design and the remaining
phase (a full check of the acceptance criteria).

## Requirements

- macOS with Xcode and its iOS Simulator. This Mac has Xcode 27.1.
- Node.js and npm. This Mac has Node 26.12.0 and npm 11.20.0; the chat servers will rely on Node's
  built-in SQLite and its ability to run TypeScript directly. Node is a host prerequisite: the
  repository's setup scripts install only Rust.
- For an iPhone: an Apple ID signed in to Xcode with a signing team, Developer Mode turned on in
  the iPhone's settings, and the iPhone on the same Wi-Fi network as the Mac.

## Set up

```bash
cd apps/paracels
npm ci
```

## Run on the iOS Simulator

```bash
npx expo run:ios
```

This generates and builds the native project, installs the app in the simulator, starts the dev
server and opens the app. Run it again after adding a package with native code or changing
`app.json`; JavaScript changes reload without a rebuild.

## Run on an iPhone

1. Connect the iPhone with USB the first time, unlock it and trust the Mac.
2. If Developer Mode is off, turn it on: Settings, then Privacy & Security, then Developer Mode.
3. Build and install:

   ```bash
   npx expo run:ios --device
   ```

   Pick your iPhone from the list. Xcode signs the build with your team. Keep the iPhone unlocked
   until the app opens: if the phone locks during the build, Expo CLI stops with "the device is
   locked". Unlock it and run the command again; the second build reuses the first one's work.
4. When the app asks to find devices on your local network, allow it. The app loads its JavaScript
   from the dev server on the Mac, and the chat will talk to servers there.

With a free Apple ID team, iOS first asks you to trust the developer (Settings, then General, then
VPN & Device Management), and the install expires after seven days.

Phase 1 of the chat plan tested these steps on 2026-10-22 with an iPhone 16 Pro.

## Checks

```bash
npx tsc --noEmit   # typecheck
npm run lint       # lint src/, server/ and scripts/
npm test           # the chat protocol, servers and scripts
```

## Chat prototype

Status: the servers, the chat screen, the on-device outbox and attachments are built (Phases 2 through 5).
Gate G3, chat between the simulator and the iPhone, is closed. Gate G4 is airplane mode, force-quit
and reopen, and a socket restart. Gate G5 is sending a library photo, a camera photo and a file in
both directions. The [spec](../../docs/paracels-chat.md) holds the decisions, the design and the last phase.
A session runs two servers on this Mac:

```bash
npm run chat:setup    # once: creates .env with a room key, and does not print it
npm run chat:socket   # the WebSocket server, on port 8787
npx expo start        # the dev server: app bundle and API routes, on port 8081
npm run chat:log      # print the stored messages
npm run chat:reset    # delete the database; stop both servers first
```

Restart the dev server after `chat:setup`. It reads `.env` at startup, and until then the API
routes answer 503.

Keep `chat:socket` and `npx expo start` running while you use the app. The header menu switches
between Alice and Bob on that device. iOS may ask "Open in paracels?" when the bundle URL arrives;
Open or Cancel leaves you on the chat screen.

## Project layout

```text
app.json          app configuration: name, bundle ID, plugins; native settings go here, not in ios/
.env.example     room-key variable; the real .env is git-ignored
eslint.config.js  ESLint: Expo's rules, plus the rules that keep server code in its place
server/           the WebSocket server
src/app/          the chat screen and the API routes (+api.ts files)
src/chat/         protocol, client logic, server storage and handlers
src/components/   chat UI and shared theme wrappers
src/constants/    theme: colors, fonts, spacing
src/hooks/        color scheme and theme hooks
assets/           icons and images
scripts/          chat:setup, chat:log, chat:reset
```

## Notes

- `ios/` is generated (Continuous Native Generation) and git-ignored; configure native behavior in
  `app.json`. It sets the bundle ID `com.paracels.conclave` and the signing team `BZ5NJV27J6`
  (decision D28 in the spec), so `npx expo prebuild --clean --platform ios` regenerates `ios/`
  without losing either.
- Expo changes between SDK releases. Read the SDK 58 documentation before using an Expo API; see
  `AGENTS.md`.
