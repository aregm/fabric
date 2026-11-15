This is an Expo/React Native mobile application. Prioritize mobile-first patterns, performance, and cross-platform compatibility.

## Expo has changed — do not trust your training data

Expo ships breaking changes every SDK release. APIs you remember are likely renamed, moved, or removed. Before writing any code that touches an Expo, EAS, or React Native API:

1. Read the major version of the `expo` package in `package.json`.
2. Fetch the matching versioned docs: `https://docs.expo.dev/versions/v<major>.0.0/`
3. For anything else, fetch https://docs.expo.dev/llms.txt — an index of all Expo docs with corrections to common LLM misconceptions. Follow its links to the specific page you need; never answer from memory.

## Commands

Use `bunx` instead of `npx` if the project uses bun (`bun.lock` present).

```bash
npx expo install <package>  # ALWAYS use instead of npm/yarn/pnpm/bun add — resolves SDK-compatible versions
npx expo start              # start the dev server
npm run lint                # lint src/, server/ and scripts/
npx tsc --noEmit            # typecheck
npx expo-doctor             # diagnose dependency and config issues
npx expo install --fix      # fix incompatible package versions
```

Run lint and typecheck before declaring any task done.

## Navigation & Routing

- Use **Expo Router** for all navigation. Routes live in `src/app/` — every file there is a screen, `_layout.tsx` files define navigators. Keep non-route code (components, hooks, utils) outside `src/app/`.
- Import `Link`, `router`, and `useLocalSearchParams` from `expo-router`.
- Docs: https://docs.expo.dev/router/introduction.md

## Building with EAS

Use EAS to build, sign, and submit the app in the cloud (`eas build`, `eas submit`) and to ship over-the-air updates (`eas update`) — no local Xcode or Android Studio required. Run EAS CLI as `bunx eas-cli <command>` in Bun projects, or `npx eas-cli@latest <command>` otherwise; substitute that for bare `eas` in docs examples.
Docs: https://docs.expo.dev/eas/index.md

## Rules

- If `ios/` and `android/` directories do not exist, they are generated (Continuous Native Generation). Never create or edit them by hand — configure native behavior in `app.json` and config plugins.
- Expo Go only includes its bundled native modules. After adding a library with native code, the app needs a development build: `npx expo run:ios|android` locally, or `eas build --profile development`.
- Prefer recommended Expo modules over third-party libraries, and check your available skills before adding dependencies. Docs: https://docs.expo.dev/versions/latest/index.md

## Chat prototype

The chat is specified in [docs/paracels-chat.md](../../docs/paracels-chat.md), the source of truth for its protocol, storage, limits and phases. It was built phase by phase, with the user's approval at each gate; all six phases are done and decisions D1–D33 are all approved. When you change behavior the spec describes, update the spec in the same commit.

Where code goes:

- `src/chat/shared/`: protocol types, validators, limits and demo users. Pure TypeScript, with no React Native, Expo or Node imports.
- `src/chat/server/`: SQLite access, the message and attachment stores, the room-key check, the policy module and the HTTP handlers. Only API routes and the socket server use it.
- `src/chat/client/`: app logic, meaning server addresses, the HTTP and WebSocket clients, the outbox, attachments and chat state.
- `src/components/chat/`: chat UI. `src/app/index.tsx` is the chat screen; `src/app/api/` holds thin API routes over `src/chat/server/http.ts`.
- `server/chat-socket.ts`: the WebSocket server.

Rules:

- App code never imports `src/chat/server/` or a `node:` module. Don't use the `server-only` package as a guard: plain Node throws when it loads it.
- Plain Node runs `src/chat/shared/`, `src/chat/server/` and `server/` as well as Metro. Use erasable TypeScript only (no `enum`, `namespace` or constructor parameter properties), relative imports ending in `.ts`, and `import type` for anything used only as a type; no `@/` aliases there. Node refuses each of these otherwise (spec section 17, Phase 1 results).
- Give every server-side WebSocket connection an `error` handler. `ws` raises an error for a frame over the size limit, and an unhandled one stops the whole socket server.
- Only the socket server writes the `messages` table; only the API routes write `attachments`.
- On the server, only `src/chat/server/policy.ts` decides whether a message or upload is acceptable, and only it reads payload fields.
- Message and attachment IDs are UUIDs created on the device with `expo-crypto`'s `randomUUID()`. A retry resends the same ID; never create a new ID for the same message.
- A change to a frame or the payload needs a new version (`paracels-chat/1`, `plain/1`). The server rejects what it doesn't understand; the app ignores server frames it doesn't know.
- Render message text with `<Text>` only: no HTML, Markdown or link rendering.
- Never log message text, file contents or the room key. Never commit `.env` or `.local/`.
- Functional modules, no classes; descriptive names; JSDoc on exported functions; an accessibility label on every interactive element.
- Native packages need a rebuilt app (`npx expo run:ios`), and the iOS Simulator has no camera.

Commands:

```bash
npm run chat:setup    # create .env with a fresh room key if it is missing
npm run chat:socket   # start the WebSocket server on port 8787
npx expo start        # dev server: app bundle and API routes on port 8081
npm test              # shared, server and client-logic tests (node --test)
npm run chat:log      # print the stored messages
npm run chat:reset    # delete the chat database (stop both servers first)
```
