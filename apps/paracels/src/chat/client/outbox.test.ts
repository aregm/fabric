import assert from 'node:assert/strict';
import { describe, it } from 'node:test';

import { createMemoryOutbox, createOutboxWorker, type OutboxItem } from './outbox.ts';

function item(id: string, state: OutboxItem['state'] = 'waiting', created = '2026-10-26T05:00:00.000Z'): OutboxItem {
  return { id, senderId: 'alice', clientCreatedAt: created, text: 'hi', attachment: null, state, failureCode: null, attempts: 0 };
}

describe('outbox worker', () => {
  it('sends the oldest waiting message and removes it on ack', () => {
    const store = createMemoryOutbox();
    store.put(item('b', 'waiting', '2026-10-26T05:00:02.000Z'));
    store.put(item('a', 'waiting', '2026-10-26T05:00:01.000Z'));
    store.put(item('f', 'failed', '2026-10-26T04:00:00.000Z'));
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
    assert.equal(store.get('a')?.state, 'sending');
    worker.handleAck('a');
    assert.equal(store.get('a'), undefined);
    assert.deepEqual(sent, ['a', 'b']);
    worker.dispose();
  });

  it('marks a reject failed and continues with the next waiting message', () => {
    const store = createMemoryOutbox();
    store.put(item('a'));
    store.put(item('b', 'waiting', '2026-10-26T05:00:01.000Z'));
    const worker = createOutboxWorker({
      store,
      isConnected: () => true,
      send: () => undefined,
      onChange: () => undefined,
      ackTimeoutMs: 10_000,
    });
    worker.kick();
    worker.handleReject('a', 'unknown_sender');
    assert.equal(store.get('a')?.state, 'failed');
    assert.equal(store.get('a')?.failureCode, 'unknown_sender');
    assert.equal(store.get('b')?.state, 'sending');
    worker.dispose();
  });

  it('returns a sending message to waiting when the connection drops', () => {
    const store = createMemoryOutbox();
    store.put(item('a'));
    const worker = createOutboxWorker({
      store,
      isConnected: () => true,
      send: () => undefined,
      onChange: () => undefined,
      ackTimeoutMs: 10_000,
    });
    worker.kick();
    worker.handleDisconnect();
    assert.equal(store.get('a')?.state, 'waiting');
    worker.dispose();
  });

  it('marks a message failed when no ack arrives in time', () => {
    const store = createMemoryOutbox();
    store.put(item('a'));
    let now = 0;
    const scheduled: { work: () => void }[] = [];
    const worker = createOutboxWorker({
      store,
      isConnected: () => true,
      send: () => undefined,
      onChange: () => undefined,
      ackTimeoutMs: 10_000,
      now: () => now,
      schedule: (work) => {
        scheduled.push({ work });
        return () => undefined;
      },
    });
    worker.kick();
    now = 10_000;
    scheduled[0]?.work();
    assert.equal(store.get('a')?.state, 'failed');
    assert.equal(store.get('a')?.failureCode, 'ack_timeout');
    worker.dispose();
  });

  it('uploads an attachment before sending, and uploads once more after attachment_missing', async () => {
    const store = createMemoryOutbox();
    store.put(withFile('a'));
    let uploads = 0;
    const sent: string[] = [];
    const worker = createOutboxWorker({
      store,
      isConnected: () => true,
      send: (next) => sent.push(next.attachment?.sha256 ?? ''),
      onChange: () => undefined,
      ackTimeoutMs: 10_000,
      prepare: async (next) => {
        uploads += 1;
        return { ok: true, item: { ...next, attachment: { ...next.attachment!, sha256: 'ab'.repeat(32) } } };
      },
    });
    worker.kick();
    await flush();
    assert.equal(uploads, 1);
    assert.equal(sent.length, 1);
    worker.handleReject('a', 'attachment_missing');
    await flush();
    assert.equal(uploads, 2);
    worker.handleReject('a', 'attachment_missing');
    await flush();
    assert.equal(store.get('a')?.state, 'failed');
    assert.equal(uploads, 2);
    worker.dispose();
  });

  it('leaves the message waiting and tries the upload again after a network failure', async () => {
    const store = createMemoryOutbox();
    store.put(withFile('a'));
    store.put(withFile('b', '2026-10-26T05:00:01.000Z'));
    const scheduled: { work: () => void }[] = [];
    let uploads = 0;
    const sent: string[] = [];
    const worker = createOutboxWorker({
      store,
      isConnected: () => true,
      send: (next) => sent.push(next.id),
      onChange: () => undefined,
      ackTimeoutMs: 10_000,
      schedule: (work) => {
        scheduled.push({ work });
        return () => undefined;
      },
      prepare: async () => {
        uploads += 1;
        return { ok: false, retryable: true, code: 'network' };
      },
    });
    worker.kick();
    await flush();
    assert.equal(store.get('a')?.state, 'waiting');
    assert.equal(sent.length, 0);
    assert.equal(scheduled.length, 1);
    scheduled[0]?.work();
    await flush();
    assert.equal(uploads, 2);
    worker.dispose();
  });
});

function withFile(id: string, created = '2026-10-26T05:00:00.000Z'): OutboxItem {
  return {
    ...item(id, 'waiting', created),
    attachment: { id: `${id}-file`, uri: 'file:///tmp/a', fileName: 'a.txt', mimeType: 'text/plain', byteSize: 4, sha256: null },
  };
}

async function flush(): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, 0));
}
