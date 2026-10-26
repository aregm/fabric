/**
 * The device outbox as SQL (docs/paracels-chat.md, sections 6.3 and 9).
 * `expo-sqlite` and the tests both speak {@link SqlConnection}, so the schema
 * and the recovery rule can run without the app.
 */

import { isDemoUserId } from '../shared/users.ts';
import type { OutboxItem, OutboxState, OutboxStore } from './outbox.ts';

/** The on-device database file (section 5.5). */
export const OUTBOX_DATABASE_NAME = 'paracels-chat.db';

/** The schema version this code writes and expects. */
export const OUTBOX_SCHEMA_VERSION = 1;

const CREATE_OUTBOX = `
  CREATE TABLE outbox (
    id                TEXT PRIMARY KEY,
    sender_id         TEXT NOT NULL,
    client_created_at TEXT NOT NULL,
    text              TEXT NOT NULL,
    attachment_id     TEXT,
    attachment_uri    TEXT,
    attachment_name   TEXT,
    attachment_mime   TEXT,
    attachment_size   INTEGER,
    attachment_sha256 TEXT,
    state             TEXT NOT NULL,
    failure_code      TEXT,
    attempts          INTEGER NOT NULL DEFAULT 0
  );
`;

export type SqlValue = string | number | null;

/**
 * The few SQLite operations the outbox uses. Both `expo-sqlite` and Node's
 * `node:sqlite` can satisfy it.
 */
export type SqlConnection = {
  exec(source: string): void;
  getAll<T>(source: string, params?: readonly SqlValue[]): T[];
  getFirst<T>(source: string, params?: readonly SqlValue[]): T | null;
  run(source: string, params?: readonly SqlValue[]): void;
  transaction(task: () => void): void;
};

type OutboxRow = {
  id: string;
  sender_id: string;
  client_created_at: string;
  text: string;
  state: string;
  failure_code: string | null;
  attempts: number;
};

const LIST_SQL = `
  SELECT id, sender_id, client_created_at, text, state, failure_code, attempts
  FROM outbox
  ORDER BY client_created_at ASC, id ASC
`;

const UPSERT_SQL = `
  INSERT INTO outbox (
    id, sender_id, client_created_at, text,
    attachment_id, attachment_uri, attachment_name, attachment_mime, attachment_size, attachment_sha256,
    state, failure_code, attempts
  ) VALUES (?, ?, ?, ?, NULL, NULL, NULL, NULL, NULL, NULL, ?, ?, ?)
  ON CONFLICT(id) DO UPDATE SET
    sender_id = excluded.sender_id,
    client_created_at = excluded.client_created_at,
    text = excluded.text,
    state = excluded.state,
    failure_code = excluded.failure_code,
    attempts = excluded.attempts
`;

/**
 * Creates the outbox table when this is a new file, and refuses a schema this
 * app does not know.
 * @param connection An open database.
 */
export function migrateOutbox(connection: SqlConnection): void {
  connection.exec('PRAGMA foreign_keys = ON');
  connection.exec('PRAGMA journal_mode = WAL');
  const version = readUserVersion(connection);
  if (version === OUTBOX_SCHEMA_VERSION) {
    return;
  }
  if (version > OUTBOX_SCHEMA_VERSION) {
    throw new Error(`The outbox database has schema version ${version}; this app knows up to ${OUTBOX_SCHEMA_VERSION}.`);
  }
  connection.transaction(() => {
    if (readUserVersion(connection) < 1) {
      connection.exec(CREATE_OUTBOX);
    }
    connection.exec(`PRAGMA user_version = ${OUTBOX_SCHEMA_VERSION}`);
  });
}

/**
 * Opens an outbox on a connection that is already open.
 * A row left in `sending` because the app closed mid-send goes back to `waiting`.
 * @param connection An open database. This function migrates it.
 */
export function createSqlOutbox(connection: SqlConnection): OutboxStore {
  migrateOutbox(connection);
  connection.run(`UPDATE outbox SET state = 'waiting', failure_code = NULL WHERE state = 'sending'`);
  return {
    list() {
      return connection.getAll<OutboxRow>(LIST_SQL).flatMap((row) => {
        const item = readItem(row);
        return item === undefined ? [] : [item];
      });
    },
    get(id) {
      return this.list().find((item) => item.id === id);
    },
    put(item) {
      connection.run(UPSERT_SQL, [
        item.id,
        item.senderId,
        item.clientCreatedAt,
        item.text,
        item.state,
        item.failureCode,
        item.attempts,
      ]);
    },
    remove(id) {
      connection.run('DELETE FROM outbox WHERE id = ?', [id]);
    },
  };
}

function readUserVersion(connection: SqlConnection): number {
  const row = connection.getFirst<{ user_version: number | bigint }>('PRAGMA user_version');
  const version = row?.user_version ?? 0;
  const numeric = typeof version === 'bigint' ? Number(version) : version;
  return Number.isInteger(numeric) ? numeric : 0;
}

function readItem(row: OutboxRow): OutboxItem | undefined {
  if (!isDemoUserId(row.sender_id) || !isOutboxState(row.state)) {
    return undefined;
  }
  if (typeof row.id !== 'string' || typeof row.client_created_at !== 'string' || typeof row.text !== 'string') {
    return undefined;
  }
  if (!Number.isInteger(row.attempts) || row.attempts < 0) {
    return undefined;
  }
  if (row.failure_code !== null && typeof row.failure_code !== 'string') {
    return undefined;
  }
  return {
    id: row.id,
    senderId: row.sender_id,
    clientCreatedAt: row.client_created_at,
    text: row.text,
    state: row.state,
    failureCode: row.failure_code,
    attempts: row.attempts,
  };
}

function isOutboxState(value: string): value is OutboxState {
  return value === 'waiting' || value === 'sending' || value === 'failed';
}
