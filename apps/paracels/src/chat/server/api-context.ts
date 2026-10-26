/**
 * The API routes' side of the server (docs/paracels-chat.md, section 5.2): one
 * database connection per dev server process, reused across requests and code
 * reloads, and settings read from the environment Expo CLI loads from `.env`.
 */

import { createAttachmentReader, createAttachmentWriter } from './attachment-store.ts';
import { readRoomKey, resolveDatabasePath } from './config.ts';
import { openChatDatabase, type ChatDatabase } from './database.ts';
import { internalErrorResponse, type HttpContext } from './http.ts';
import { createJsonLogger } from './log.ts';
import { createMessageReader } from './message-store.ts';
import { createRoomKeyCheck } from './room-key.ts';

/** A code reload re-runs this module but keeps `globalThis`, and with it the connection. */
const DATABASES_KEY = '__paracelsChatDatabases';

type GlobalWithDatabases = typeof globalThis & { [DATABASES_KEY]?: Map<string, ChatDatabase> };

type CachedContext = { databasePath: string; roomKey: string | null; context: HttpContext };

let cachedContext: CachedContext | null = null;

const log = createJsonLogger((line) => console.log(line));

/**
 * Runs one API route handler with the shared context, turning any unexpected
 * error into a 500 response that reveals nothing about it.
 * @param handle The handler, given the context.
 */
export async function runApiHandler(handle: (context: HttpContext) => Promise<Response>): Promise<Response> {
  try {
    return await handle(getApiContext());
  } catch (error) {
    log('http_error', { error: error instanceof Error ? error.message : 'unknown error' });
    return internalErrorResponse();
  }
}

/**
 * Returns the context for this process, rebuilding it when `CHAT_DB_PATH` or the
 * room key changes.
 */
export function getApiContext(): HttpContext {
  const databasePath = resolveDatabasePath(process.env, process.cwd());
  const roomKey = readRoomKey(process.env);
  if (cachedContext?.databasePath === databasePath && cachedContext.roomKey === roomKey) {
    return cachedContext.context;
  }
  const database = getSharedDatabase(databasePath);
  const context: HttpContext = {
    isRoomKey: roomKey === null ? null : createRoomKeyCheck(roomKey),
    databaseId: database.databaseId,
    messages: createMessageReader(database),
    attachments: { ...createAttachmentReader(database), ...createAttachmentWriter(database) },
    log,
    now: () => new Date(),
  };
  cachedContext = { databasePath, roomKey, context };
  return context;
}

function getSharedDatabase(databasePath: string): ChatDatabase {
  const holder: GlobalWithDatabases = globalThis;
  const databases = (holder[DATABASES_KEY] ??= new Map());
  const existing = databases.get(databasePath);
  if (existing !== undefined && existing.connection.isOpen) {
    return existing;
  }
  const database = openChatDatabase(databasePath);
  databases.set(databasePath, database);
  return database;
}
