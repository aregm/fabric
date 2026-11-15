/**
 * Creates `apps/paracels/.env` with a room key, if the file is not already there
 * (docs/paracels-chat.md, D27). The key is never printed.
 */

import { randomBytes } from 'node:crypto';
import { writeFileSync } from 'node:fs';

import { ROOM_KEY_VARIABLE } from '../src/chat/server/config.ts';

/** What creating the env file did. */
export type RoomKeyFileStatus = 'created' | 'exists';

/**
 * Writes a room key into a new env file. An existing file is left untouched,
 * so a second run cannot replace a key the app is already using.
 * @param filePath Where the env file should be.
 * @param createKey Generates the key. Tests pass their own.
 * @returns Whether the file was created.
 */
export function ensureRoomKeyFile(filePath: string, createKey: () => string = createRoomKey): RoomKeyFileStatus {
  const contents = `${ROOM_KEY_VARIABLE}=${createKey()}\n`;
  try {
    writeFileSync(filePath, contents, { encoding: 'utf8', mode: 0o600, flag: 'wx' });
    return 'created';
  } catch (error) {
    if (isAlreadyExists(error)) {
      return 'exists';
    }
    throw error;
  }
}

/**
 * Runs setup, reporting the outcome without the key.
 * @param filePath Where the env file should be.
 * @param writeLine Receives each line of output.
 * @param createKey Generates the key. Tests pass their own.
 */
export function runChatSetup(filePath: string, writeLine: (line: string) => void, createKey?: () => string): void {
  const status = ensureRoomKeyFile(filePath, createKey);
  writeLine(
    status === 'created'
      ? `Wrote a room key to ${filePath}. It was not printed. Restart the dev server so it picks the key up.`
      : `Left ${filePath} unchanged because it already exists.`,
  );
}

/** A room key: 256 random bits, encoded so it fits in a header and an env file. */
function createRoomKey(): string {
  return randomBytes(32).toString('base64url');
}

function isAlreadyExists(error: unknown): boolean {
  return typeof error === 'object' && error !== null && 'code' in error && error.code === 'EEXIST';
}

if (import.meta.main) {
  runChatSetup('.env', (line) => {
    process.stdout.write(`${line}\n`);
  });
}
