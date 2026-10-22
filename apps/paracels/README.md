# Paracels

An Expo (React Native) app for iOS, built with Expo SDK 58 and Expo Router. It lives in the Fabric
repository but is not part of the Cargo workspace.

Today it is the create-expo-app starter, plus the packages and tooling the chat needs. The
[chat prototype spec](../../docs/paracels-chat.md) describes the dummy chat that will replace the
starter screens and the phases that build it.

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
npx expo lint      # lint (ESLint with eslint-config-expo; checks src/)
npm test           # node --test; the tests arrive with Phase 2
```

## Chat prototype (planned)

Status: specified, with Phase 1 (packages, tooling and spikes) done; the chat itself is not built
yet. The [spec](../../docs/paracels-chat.md) holds the decisions, the design, the phased plan and
the spike results. Once built, a chat session runs two servers on this Mac:

```bash
npm run chat:setup    # once: creates .env with a room key
npm run chat:socket   # the WebSocket server, on port 8787
npx expo start        # the dev server: app bundle and API routes, on port 8081
```

The `chat:` scripts don't exist yet; Phase 2 adds them.

## Project layout

```text
app.json          app configuration: name, bundle ID, plugins; native settings go here, not in ios/
eslint.config.js  ESLint: Expo's rules, plus the rule that keeps server-only imports out of the app
src/app/          routes: screens, and later the API routes (+api.ts files)
src/components/   UI components
src/constants/    theme: colors, fonts, spacing
src/hooks/        color scheme and theme hooks
assets/           icons and images
scripts/          reset-project.js from the starter
```

## Notes

- `ios/` is generated (Continuous Native Generation) and git-ignored; configure native behavior in
  `app.json`. It sets the bundle ID `com.paracels.conclave` and the signing team `BZ5NJV27J6`
  (decision D28 in the spec), so `npx expo prebuild --clean --platform ios` regenerates `ios/`
  without losing either.
- Expo changes between SDK releases. Read the SDK 58 documentation before using an Expo API; see
  `AGENTS.md`.
