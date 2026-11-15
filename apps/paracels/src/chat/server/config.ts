/**
 * Server settings read from the environment (docs/paracels-chat.md, D1, D2 and D27).
 * Both servers and the chat scripts start in `apps/paracels`, so relative paths
 * resolve to the same files.
 */

import { resolve } from 'node:path';

import { DEFAULT_SOCKET_PORT } from '../shared/protocol.ts';
import type { Validation } from '../shared/validate.ts';

export type Environment = Readonly<Record<string, string | undefined>>;

/** Where the database lives unless `CHAT_DB_PATH` says otherwise (D2). */
export const DEFAULT_DATABASE_PATH = '.local/chat.sqlite';

/** The variable that holds the room key, for the app and both servers (D27). */
export const ROOM_KEY_VARIABLE = 'EXPO_PUBLIC_CHAT_ROOM_KEY';

/**
 * Resolves the database file from `CHAT_DB_PATH`, or the default.
 * @param environment Usually `process.env`.
 * @param workingDirectory The directory relative paths start from.
 */
export function resolveDatabasePath(environment: Environment, workingDirectory: string): string {
  const configuredPath = environment.CHAT_DB_PATH?.trim();
  return resolve(workingDirectory, configuredPath ? configuredPath : DEFAULT_DATABASE_PATH);
}

/**
 * Reads the room key.
 * @param environment Usually `process.env`.
 * @returns The key, or `null` when it is missing or blank.
 */
export function readRoomKey(environment: Environment): string | null {
  const roomKey = environment[ROOM_KEY_VARIABLE]?.trim();
  return roomKey ? roomKey : null;
}

/**
 * Reads the socket server's port from `CHAT_SOCKET_PORT`, or the default.
 * @param environment Usually `process.env`.
 */
export function resolveSocketPort(environment: Environment): Validation<number> {
  const configuredPort = environment.CHAT_SOCKET_PORT?.trim();
  if (!configuredPort) {
    return { ok: true, value: DEFAULT_SOCKET_PORT };
  }
  const port = /^\d{1,5}$/.test(configuredPort) ? Number(configuredPort) : Number.NaN;
  if (!(port >= 1 && port <= 65535)) {
    return { ok: false, detail: 'CHAT_SOCKET_PORT must be a port number from 1 to 65535.' };
  }
  return { ok: true, value: port };
}
