import assert from 'node:assert/strict';
import { mkdtempSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { DatabaseSync } from 'node:sqlite';
import { describe, it } from 'node:test';

import type { OutboxItem } from './outbox.ts';
import { createOutboxWorker } from './outbox.ts';
import {
  createSqlOutbox,
  OUTBOX_SCHEMA_VERSION,
  type SqlConnection,
  type SqlValue,
} from './outbox-sql.ts';

function item(id: string, state: OutboxItem['state'] = 'waiting', created = '2026-10-26T05:00:00.000Z'): OutboxItem {
  return { id, senderId: 'alice', clientCreatedAt: created, text: 'hi', attachment: null, state, failureCode: null, attempts: 0 };
}

function openFile(t: { after: (fn: () => void) => void }, filePath: string): SqlConnection {
  const database = new DatabaseSync(filePath);
  t.after(() => database.close());
  return {
    exec(source) {
      database.exec(source);
    },
    getAll(source, params = []) {
      return database.prepare(source).all(...params) as never[];
    },
    getFirst<T>(source: string, params: readonly SqlValue[] = []) {
      return (database.prepare(source).get(...params) as T | undefined) ?? null;
    },
    run(source, params = []) {
      database.prepare(source).run(...params);
    },
    transaction(task) {
      database.exec('BEGIN IMMEDIATE');
      try {
        task();
        database.exec('COMMIT');
      } catch (error) {
        if (database.isTransaction) {
          database.exec('ROLLBACK');
        }
        throw error;
      }
    },
  };
}

describe('sqlite outbox', () => {
  it('creates the schema, keeps rows across a reopen, and returns an interrupted send to waiting', (t) => {
    const filePath = join(mkdtempSync(join(tmpdir(), 'paracels-outbox-')), 'paracels-chat.db');
    const first = createSqlOutbox(openFile(t, filePath));
    first.put(item('keep', 'waiting', '2026-10-26T05:00:02.000Z'));
    first.put({
      ...item('file', 'waiting', '2026-10-26T05:00:03.000Z'),
      attachment: {
        id: '00000000-0000-4000-8000-000000000099',
        uri: 'file:///outbox/a.txt',
        fileName: 'a.txt',
        mimeType: 'text/plain',
        byteSize: 4,
        sha256: 'ab'.repeat(32),
      },
    });
    first.put({ ...item('mid', 'sending', '2026-10-26T05:00:01.000Z'), attempts: 2, failureCode: 'ack_timeout' });
    first.put(item('gone'));
    first.remove('gone');

    const second = createSqlOutbox(openFile(t, filePath));
    assert.deepEqual(
      second.list().map((entry) => entry.id),
      ['mid', 'keep', 'file'],
    );
    assert.equal(second.get('file')?.attachment?.fileName, 'a.txt');
    assert.equal(second.get('file')?.attachment?.sha256, 'ab'.repeat(32));
    assert.equal(second.get('mid')?.state, 'waiting');
    assert.equal(second.get('mid')?.failureCode, null);
    assert.equal(second.get('mid')?.attempts, 2);
    assert.equal(second.get('gone'), undefined);

    const version = openFile(t, filePath).getFirst<{ user_version: number }>('PRAGMA user_version');
    assert.equal(Number(version?.user_version), OUTBOX_SCHEMA_VERSION);
  });

  it('refuses a schema newer than this app', (t) => {
    const filePath = join(mkdtempSync(join(tmpdir(), 'paracels-outbox-')), 'paracels-chat.db');
    const connection = openFile(t, filePath);
    connection.exec(`PRAGMA user_version = ${OUTBOX_SCHEMA_VERSION + 1}`);
    assert.throws(() => createSqlOutbox(connection), /schema version/);
  });

  it('flushes the oldest waiting row and deletes it on ack', (t) => {
    const filePath = join(mkdtempSync(join(tmpdir(), 'paracels-outbox-')), 'paracels-chat.db');
    const store = createSqlOutbox(openFile(t, filePath));
    store.put(item('b', 'waiting', '2026-10-26T05:00:02.000Z'));
    store.put(item('a', 'waiting', '2026-10-26T05:00:01.000Z'));
    const sent: string[] = [];
    const worker = createOutboxWorker({
      store,
      isConnected: () => true,
      send: (next) => sent.push(next.id),
      onChange: () => undefined,
      ackTimeoutMs: 10_000,
    });
    worker.kick();
    assert.deepEqual(sent, ['a']);
    worker.handleAck('a');
    assert.equal(store.get('a'), undefined);
    assert.equal(store.get('b')?.state, 'sending');
    worker.dispose();
  });
});
