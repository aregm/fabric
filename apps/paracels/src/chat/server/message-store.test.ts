import assert from 'node:assert/strict';
import { randomUUID } from 'node:crypto';
import { describe, it } from 'node:test';

import { createAttachmentWriter } from './attachment-store.ts';
import { createMessageReader, createMessageWriter } from './message-store.ts';
import { makeEnvelope, makePlainPayload, openTemporaryDatabase } from './test-helpers.ts';

const RECEIVED_AT = '2026-10-22T09:00:00.000Z';

describe('message store', () => {
  it('assigns increasing sequence numbers and keeps the receive time', (t) => {
    const writer = createMessageWriter(openTemporaryDatabase(t));
    const first = writer.storeMessage(makeEnvelope(), RECEIVED_AT);
    const second = writer.storeMessage(makeEnvelope({ senderId: 'bob' }), RECEIVED_AT);
    assert.ok(first.kind === 'stored' && second.kind === 'stored');
    assert.equal(first.message.seq, 1);
    assert.equal(second.message.seq, 2);
    assert.equal(second.message.serverReceivedAt, RECEIVED_AT);
  });

  it('stores a repeated message once and reports the repeat as a duplicate', (t) => {
    const database = openTemporaryDatabase(t);
    const writer = createMessageWriter(database);
    const envelope = makeEnvelope();
    const stored = writer.storeMessage(envelope, RECEIVED_AT);
    const repeated = writer.storeMessage(structuredClone(envelope), '2026-10-22T09:00:10.000Z');

    assert.equal(repeated.kind, 'duplicate');
    assert.deepEqual(repeated.kind === 'duplicate' && repeated.message, stored.kind === 'stored' && stored.message);
    const count = database.connection.prepare('SELECT COUNT(*) AS count FROM messages WHERE id = ?').get(envelope.id);
    assert.equal(count?.count, 1);
  });

  it('refuses a different message under an existing ID', (t) => {
    const writer = createMessageWriter(openTemporaryDatabase(t));
    const envelope = makeEnvelope();
    writer.storeMessage(envelope, RECEIVED_AT);
    const variants = [
      { ...envelope, payload: makePlainPayload('Edited') },
      { ...envelope, senderId: 'bob' },
      { ...envelope, clientCreatedAt: '2026-10-22T08:00:00.000Z' },
    ];
    for (const variant of variants) {
      assert.deepEqual(writer.storeMessage(variant, RECEIVED_AT), { kind: 'conflict' });
    }
  });

  it('stores the payload verbatim', (t) => {
    const database = openTemporaryDatabase(t);
    const payload = makePlainPayload('  spaces, <b>markup</b> and émoji 🙂  ');
    createMessageWriter(database).storeMessage(makeEnvelope({ payload }), RECEIVED_AT);
    const row = database.connection.prepare('SELECT payload_format, payload_json FROM messages').get();
    assert.equal(row?.payload_format, 'plain/1');
    assert.equal(row?.payload_json, JSON.stringify(payload));
  });

  it('reads the newest page, then older and newer pages, in ascending order', (t) => {
    const database = openTemporaryDatabase(t);
    const writer = createMessageWriter(database);
    for (let index = 0; index < 7; index += 1) {
      writer.storeMessage(makeEnvelope(), RECEIVED_AT);
    }
    const reader = createMessageReader(database);
    const read = (cursor: { before: number } | { after: number } | null, limit = 3) => {
      const page = reader.readHistory({ roomId: 'main', limit, cursor });
      return { seqs: page.messages.map((message) => message.seq), latestSeq: page.latestSeq, hasMore: page.hasMore };
    };

    assert.deepEqual(read(null), { seqs: [5, 6, 7], latestSeq: 7, hasMore: true });
    assert.deepEqual(read({ before: 5 }), { seqs: [2, 3, 4], latestSeq: 7, hasMore: true });
    assert.deepEqual(read({ before: 2 }), { seqs: [1], latestSeq: 7, hasMore: false });
    assert.deepEqual(read({ after: 0 }), { seqs: [1, 2, 3], latestSeq: 7, hasMore: true });
    assert.deepEqual(read({ after: 4 }), { seqs: [5, 6, 7], latestSeq: 7, hasMore: false });
    assert.deepEqual(read({ after: 7 }), { seqs: [], latestSeq: 7, hasMore: false });
    assert.deepEqual(read(null, 200), { seqs: [1, 2, 3, 4, 5, 6, 7], latestSeq: 7, hasMore: false });
    assert.equal(reader.readHistory({ roomId: 'other', limit: 3, cursor: null }).latestSeq, 0);
  });

  it('returns messages in the shape the app receives', (t) => {
    const database = openTemporaryDatabase(t);
    const envelope = makeEnvelope();
    createMessageWriter(database).storeMessage(envelope, RECEIVED_AT);
    const [message] = createMessageReader(database).readHistory({ roomId: 'main', limit: 1, cursor: null }).messages;
    assert.deepEqual(message, { ...envelope, seq: 1, serverReceivedAt: RECEIVED_AT });
  });

  it('refuses a message whose attachment was never uploaded', (t) => {
    const database = openTemporaryDatabase(t);
    const writer = createMessageWriter(database);
    assert.throws(() => writer.storeMessage(makeEnvelope({ attachmentId: randomUUID() }), RECEIVED_AT), /FOREIGN KEY/);

    const attachmentId = randomUUID();
    createAttachmentWriter(database).storeAttachment({
      id: attachmentId,
      uploaderId: 'alice',
      bytes: new Uint8Array([1, 2, 3]),
      createdAt: RECEIVED_AT,
    });
    assert.equal(writer.storeMessage(makeEnvelope({ attachmentId }), RECEIVED_AT).kind, 'stored');
  });
});
