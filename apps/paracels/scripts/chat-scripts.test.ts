import assert from 'node:assert/strict';
import { chmodSync, existsSync, readFileSync, statSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, it } from 'node:test';

import { createMessageWriter } from '../src/chat/server/message-store.ts';
import { createTemporaryDirectory, makeEnvelope, makePlainPayload, openTemporaryDatabase } from '../src/chat/server/test-helpers.ts';
import { renderChatLog } from './chat-log.ts';
import { removeDatabaseFiles, runChatReset } from './chat-reset.ts';
import { ensureRoomKeyFile, runChatSetup } from './chat-setup.ts';

const RECEIVED_AT = '2026-10-22T09:00:00.000Z';

describe('chat:setup', () => {
  it('writes a room key once, and never prints it', (t) => {
    const filePath = join(createTemporaryDirectory(t), '.env');
    const lines: string[] = [];
    runChatSetup(filePath, (line) => lines.push(line), () => 'test-room-key');

    assert.equal(readFileSync(filePath, 'utf8'), 'EXPO_PUBLIC_CHAT_ROOM_KEY=test-room-key\n');
    assert.equal(statSync(filePath).mode & 0o777, 0o600);
    assert.equal(lines.join('\n').includes('test-room-key'), false);
    assert.match(lines[0], /not printed/);

    const rewritten: string[] = [];
    const again = runChatSetup(filePath, (line) => rewritten.push(line), () => 'replacement-key');
    assert.equal(again, undefined);
    assert.equal(readFileSync(filePath, 'utf8'), 'EXPO_PUBLIC_CHAT_ROOM_KEY=test-room-key\n');
    assert.equal(rewritten.join('\n').includes('test-room-key'), false);
    assert.equal(rewritten.join('\n').includes('replacement-key'), false);
    assert.match(rewritten[0], /unchanged/);
  });

  it('reports an existing file without reading it into the output', (t) => {
    const filePath = join(createTemporaryDirectory(t), '.env');
    writeFileSync(filePath, 'EXPO_PUBLIC_CHAT_ROOM_KEY=already-there\n');
    chmodSync(filePath, 0o600);
    assert.equal(ensureRoomKeyFile(filePath, () => 'new-key'), 'exists');
    assert.equal(readFileSync(filePath, 'utf8'), 'EXPO_PUBLIC_CHAT_ROOM_KEY=already-there\n');
  });
});

describe('chat:reset', () => {
  it('deletes the database and its journal files', (t) => {
    const directory = createTemporaryDirectory(t);
    const filePath = join(directory, 'chat.sqlite');
    writeFileSync(filePath, 'db');
    writeFileSync(`${filePath}-wal`, 'wal');
    writeFileSync(`${filePath}-shm`, 'shm');
    const lines: string[] = [];
    runChatReset(filePath, (line) => lines.push(line));
    assert.equal(existsSync(filePath), false);
    assert.equal(existsSync(`${filePath}-wal`), false);
    assert.equal(existsSync(`${filePath}-shm`), false);
    assert.match(lines[0], /Stop the dev server and the socket server/);
    assert.equal(lines.slice(1).some((line) => line.includes(filePath)), true);
  });

  it('says so when there is nothing to delete', (t) => {
    const filePath = join(createTemporaryDirectory(t), 'chat.sqlite');
    assert.deepEqual(removeDatabaseFiles(filePath), []);
    const lines: string[] = [];
    runChatReset(filePath, (line) => lines.push(line));
    assert.match(lines[1], /No chat database/);
  });
});

describe('chat:log', () => {
  it('prints stored messages and hides payloads it cannot read', (t) => {
    const database = openTemporaryDatabase(t);
    const writer = createMessageWriter(database);
    writer.storeMessage(makeEnvelope({ payload: makePlainPayload('Hello from Alice'), senderId: 'alice' }), RECEIVED_AT);
    writer.storeMessage(
      makeEnvelope({
        senderId: 'bob',
        attachmentId: null,
        payload: { format: 'e2ee/1', ciphertext: 'secret-ciphertext' },
      }),
      RECEIVED_AT,
    );
    database.connection.close();

    const text = renderChatLog(database.filePath);
    assert.equal(
      text,
      ['#1  2026-10-22T09:00:00.000Z  Alice', 'Hello from Alice', '', '#2  2026-10-22T09:00:00.000Z  Bob', '(unsupported payload)'].join(
        '\n',
      ),
    );
    assert.equal(text.includes('secret-ciphertext'), false);
  });

  it('prints an attachment name', (t) => {
    const database = openTemporaryDatabase(t);
    createMessageWriter(database).storeMessage(
      makeEnvelope({
        attachmentId: null,
        payload: makePlainPayload('See the plan', {
          fileName: 'plan.pdf',
          mimeType: 'application/pdf',
          byteSize: 4,
          sha256: 'ab'.repeat(32),
        }),
      }),
      RECEIVED_AT,
    );
    database.connection.close();
    const text = renderChatLog(database.filePath);
    assert.match(text, /Alice/);
    assert.match(text, /Attachment: plan\.pdf/);
    assert.match(text, /See the plan/);
  });

  it('reports a missing database and an empty one', (t) => {
    const directory = createTemporaryDirectory(t);
    const missing = join(directory, 'chat.sqlite');
    assert.equal(renderChatLog(missing), `No chat database at ${missing}.`);

    const database = openTemporaryDatabase(t);
    database.connection.close();
    assert.equal(renderChatLog(database.filePath), 'No messages stored.');
  });
});
