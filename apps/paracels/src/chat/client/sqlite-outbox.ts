/**
 * The device outbox in expo-sqlite (docs/paracels-chat.md, sections 5.5 and 6.3).
 */

import { openDatabaseSync, type SQLiteDatabase } from 'expo-sqlite';

import { createSqlOutbox, OUTBOX_DATABASE_NAME, type SqlConnection, type SqlValue } from './outbox-sql.ts';
import type { OutboxStore } from './outbox.ts';

/**
 * Opens `paracels-chat.db`, migrates it, and returns unsent messages.
 * A message left in `sending` is put back to `waiting`.
 * @param databaseName Defaults to {@link OUTBOX_DATABASE_NAME}.
 */
export function createSqliteOutbox(databaseName: string = OUTBOX_DATABASE_NAME): OutboxStore {
  return createSqlOutbox(connectionFromExpo(openDatabaseSync(databaseName)));
}

let deviceOutbox: OutboxStore | undefined;

/**
 * The one outbox for this app process. Opening it again returns the same store.
 */
export function openDeviceOutbox(): OutboxStore {
  deviceOutbox ??= createSqliteOutbox();
  return deviceOutbox;
}

function connectionFromExpo(database: SQLiteDatabase): SqlConnection {
  return {
    exec(source) {
      database.execSync(source);
    },
    getAll(source, params = []) {
      return database.getAllSync(source, bind(params));
    },
    getFirst(source, params = []) {
      return database.getFirstSync(source, bind(params)) ?? null;
    },
    run(source, params = []) {
      database.runSync(source, bind(params));
    },
    transaction(task) {
      database.withTransactionSync(task);
    },
  };
}

function bind(params: readonly SqlValue[]): SqlValue[] {
  return [...params];
}
