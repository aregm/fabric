/**
 * The chat database (docs/paracels-chat.md, sections 5.4 and 6.2). The API routes
 * and the socket server each open it through Node's built-in SQLite (D7).
 */

import { randomUUID } from 'node:crypto';
import { mkdirSync } from 'node:fs';
import { dirname } from 'node:path';
import { DatabaseSync } from 'node:sqlite';

/** An open chat database. */
export type ChatDatabase = {
  /** The SQLite connection. Only the stores in this folder run SQL on it. */
  connection: DatabaseSync;
  /** A random UUID created with the database. Clients compare it to notice a reset. */
  databaseId: string;
  /** The database file, or `:memory:`. */
  filePath: string;
};

export type OpenChatDatabaseOptions = {
  /** Opens an existing, migrated database without writing to it, as `chat:log` does. */
  readOnly?: boolean;
};

/** How long a connection waits for another process's lock, in milliseconds. */
export const BUSY_TIMEOUT_MS = 5000;

type Migration = (connection: DatabaseSync) => void;

/** Applied in order; `PRAGMA user_version` counts the ones already applied. */
const MIGRATIONS: readonly Migration[] = [
  (connection) => {
    connection.exec(`
      CREATE TABLE meta (
        key   TEXT PRIMARY KEY,
        value TEXT NOT NULL
      );

      CREATE TABLE attachments (
        id          TEXT PRIMARY KEY,
        uploader_id TEXT NOT NULL,
        byte_size   INTEGER NOT NULL,
        sha256      TEXT NOT NULL,
        bytes       BLOB NOT NULL,
        created_at  TEXT NOT NULL
      );

      CREATE TABLE messages (
        seq                INTEGER PRIMARY KEY AUTOINCREMENT,
        id                 TEXT NOT NULL UNIQUE,
        room_id            TEXT NOT NULL,
        sender_id          TEXT NOT NULL,
        client_created_at  TEXT NOT NULL,
        server_received_at TEXT NOT NULL,
        attachment_id      TEXT REFERENCES attachments(id),
        payload_format     TEXT NOT NULL,
        payload_json       TEXT NOT NULL
      );

      CREATE INDEX messages_by_room ON messages (room_id, seq);
    `);
    connection.prepare("INSERT INTO meta (key, value) VALUES ('database_id', ?)").run(randomUUID());
  },
];

/** The schema version this code writes and expects. */
export const SCHEMA_VERSION = MIGRATIONS.length;

/**
 * Opens the chat database, creating and migrating it when needed. Whichever
 * process opens it first applies the migrations; the other waits for its lock.
 * @param filePath The database file, or `:memory:` for tests.
 * @param options See {@link OpenChatDatabaseOptions}.
 * @throws When SQLite cannot open the file, or the schema is newer than this code.
 */
export function openChatDatabase(filePath: string, options: OpenChatDatabaseOptions = {}): ChatDatabase {
  const isReadOnly = options.readOnly === true;
  if (!isReadOnly && filePath !== ':memory:') {
    mkdirSync(dirname(filePath), { recursive: true });
  }
  const connection = new DatabaseSync(filePath, {
    readOnly: isReadOnly,
    timeout: BUSY_TIMEOUT_MS,
    enableForeignKeyConstraints: true,
  });
  try {
    if (isReadOnly) {
      const version = readUserVersion(connection);
      if (version !== SCHEMA_VERSION) {
        throw new Error(`The chat database has schema version ${version}; this code reads version ${SCHEMA_VERSION}.`);
      }
    } else {
      const journalMode = retryWhileBusy(() => connection.prepare('PRAGMA journal_mode = WAL').get());
      if (filePath !== ':memory:' && readText(journalMode ?? {}, 'journal_mode') !== 'wal') {
        throw new Error('The chat database could not switch to WAL mode.');
      }
      connection.exec('PRAGMA synchronous = FULL');
      migrate(connection);
    }
    return { connection, databaseId: readDatabaseId(connection), filePath };
  } catch (error) {
    connection.close();
    throw error;
  }
}

/**
 * Runs `work` inside one transaction, rolling back if it throws.
 * @param connection An open connection with no transaction in progress.
 * @param mode `IMMEDIATE` takes the write lock at once; `DEFERRED` reads one snapshot.
 * @param work Synchronous work; it must not await, so nothing else runs on the connection meanwhile.
 */
export function inTransaction<T>(connection: DatabaseSync, mode: 'DEFERRED' | 'IMMEDIATE', work: () => T): T {
  connection.exec(`BEGIN ${mode}`);
  try {
    const result = work();
    connection.exec('COMMIT');
    return result;
  } catch (error) {
    if (connection.isTransaction) {
      connection.exec('ROLLBACK');
    }
    throw error;
  }
}

/**
 * Reads a TEXT column, failing loudly if the database holds anything else.
 * @param row A row from `node:sqlite`.
 * @param column The column name.
 */
export function readText(row: Readonly<Record<string, unknown>>, column: string): string {
  const value = row[column];
  if (typeof value !== 'string') {
    throw new Error(`Column ${column} should hold text.`);
  }
  return value;
}

/**
 * Reads a TEXT column that may be NULL.
 * @param row A row from `node:sqlite`.
 * @param column The column name.
 */
export function readNullableText(row: Readonly<Record<string, unknown>>, column: string): string | null {
  return row[column] === null ? null : readText(row, column);
}

/**
 * Reads an INTEGER column as a safe JavaScript integer.
 * @param row A row from `node:sqlite`.
 * @param column The column name.
 */
export function readInteger(row: Readonly<Record<string, unknown>>, column: string): number {
  const value = row[column];
  if (typeof value !== 'number' || !Number.isSafeInteger(value)) {
    throw new Error(`Column ${column} should hold a safe integer.`);
  }
  return value;
}

/**
 * Reads a BLOB column.
 * @param row A row from `node:sqlite`.
 * @param column The column name.
 */
export function readBlob(row: Readonly<Record<string, unknown>>, column: string): Uint8Array<ArrayBuffer> {
  const value = row[column];
  if (!isByteArray(value)) {
    throw new Error(`Column ${column} should hold a blob.`);
  }
  return value;
}

/** Checks by tag rather than `instanceof`, which fails across JavaScript realms. */
function isByteArray(value: unknown): value is Uint8Array<ArrayBuffer> {
  return (
    ArrayBuffer.isView(value) &&
    Object.prototype.toString.call(value) === '[object Uint8Array]' &&
    Object.prototype.toString.call(value.buffer) === '[object ArrayBuffer]'
  );
}

const SLEEP_CELL = new Int32Array(new SharedArrayBuffer(4));

/**
 * Retries work that SQLite refused as busy without calling its busy handler,
 * which it does to avoid deadlocks: for example when several processes switch a
 * new file to WAL at once. Like the busy handler, it sleeps synchronously.
 */
function retryWhileBusy<T>(work: () => T): T {
  const deadline = Date.now() + BUSY_TIMEOUT_MS;
  for (let delay = 5; ; delay = Math.min(delay * 2, 100)) {
    try {
      return work();
    } catch (error) {
      if (!isBusyError(error) || Date.now() + delay > deadline) {
        throw error;
      }
      Atomics.wait(SLEEP_CELL, 0, 0, delay * (0.5 + Math.random()));
    }
  }
}

/** SQLITE_BUSY (5) or SQLITE_LOCKED (6), read without `instanceof`, which fails across realms. */
function isBusyError(error: unknown): boolean {
  return (
    typeof error === 'object' &&
    error !== null &&
    'errcode' in error &&
    (error.errcode === 5 || error.errcode === 6)
  );
}

function migrate(connection: DatabaseSync): void {
  inTransaction(connection, 'IMMEDIATE', () => {
    const version = readUserVersion(connection);
    if (version > SCHEMA_VERSION) {
      throw new Error(`The chat database has schema version ${version}; this code knows up to ${SCHEMA_VERSION}.`);
    }
    for (const migration of MIGRATIONS.slice(version)) {
      migration(connection);
    }
    if (version < SCHEMA_VERSION) {
      connection.exec(`PRAGMA user_version = ${SCHEMA_VERSION}`);
    }
  });
}

function readUserVersion(connection: DatabaseSync): number {
  return readInteger(connection.prepare('PRAGMA user_version').get() ?? {}, 'user_version');
}

function readDatabaseId(connection: DatabaseSync): string {
  const row = connection.prepare("SELECT value FROM meta WHERE key = 'database_id'").get();
  if (row === undefined) {
    throw new Error('The chat database has no database_id.');
  }
  return readText(row, 'value');
}
