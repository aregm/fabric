import assert from 'node:assert/strict';
import { describe, it } from 'node:test';

import { createChatApi, readUploadResponse } from './api.ts';

const TIME = '2026-10-26T05:00:00.000Z';
const MESSAGE = {
  id: '00000000-0000-4000-8000-000000000001',
  roomId: 'main',
  senderId: 'alice',
  clientCreatedAt: TIME,
  attachmentId: null,
  payload: { format: 'plain/1', text: 'Hi', attachment: null },
  seq: 1,
  serverReceivedAt: TIME,
};

describe('createChatApi', () => {
  it('reads a history page with the room key and roomId', async () => {
    const calls: { url: string; key: string | null }[] = [];
    const api = createChatApi({
      roomKey: 'the-key',
      apiOrigin: 'http://192.168.1.185:8081',
      fetch: async (input, init) => {
        const headers = new Headers(init?.headers);
        calls.push({ url: String(input), key: headers.get('Authorization') });
        return new Response(
          JSON.stringify({ databaseId: 'db-1', messages: [MESSAGE], latestSeq: 1, hasMore: false }),
          { status: 200, headers: { 'Content-Type': 'application/json' } },
        );
      },
    });
    const page = await api.readHistory({ cursor: { after: 0 }, limit: 200 });
    assert.ok(page.ok);
    assert.equal(page.value.messages[0]?.seq, 1);
    assert.equal(calls[0]?.key, 'Bearer the-key');
    assert.match(calls[0]?.url ?? '', /roomId=main/);
    assert.match(calls[0]?.url ?? '', /after=0/);
    assert.equal((calls[0]?.url ?? '').includes('the-key'), false);
  });

  it('maps a 401 to unauthorized without treating the body as messages', async () => {
    const api = createChatApi({
      roomKey: 'the-key',
      fetch: async () =>
        new Response(JSON.stringify({ error: { code: 'unauthorized', message: 'This request needs the room key.' } }), {
          status: 401,
        }),
    });
    const page = await api.readHistory();
    assert.ok(!page.ok);
    assert.equal(page.error.kind === 'http' && page.error.code, 'unauthorized');
  });
});

describe('readUploadResponse', () => {
  it('keeps the size and hash the server returns', () => {
    const sha = 'ab'.repeat(32);
    const result = readUploadResponse(201, JSON.stringify({ id: '1', byteSize: 12, sha256: sha }));
    assert.deepEqual(result, { ok: true, byteSize: 12, sha256: sha });
  });

  it('treats a conflict as terminal and a server error as retryable', () => {
    const conflict = readUploadResponse(409, JSON.stringify({ error: { code: 'attachment_conflict', message: 'Different bytes.' } }));
    assert.deepEqual(conflict, { ok: false, retryable: false, code: 'attachment_conflict' });
    const server = readUploadResponse(503, JSON.stringify({ error: { code: 'not_configured', message: 'No key.' } }));
    assert.equal(server.ok, false);
    if (!server.ok) {
      assert.equal(server.retryable, true);
    }
  });
});
