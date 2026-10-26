import assert from 'node:assert/strict';
import { createHash, randomUUID } from 'node:crypto';
import { describe, it, type TestContext } from 'node:test';

import { CHAT_LIMITS } from '../shared/limits.ts';
import { PROTOCOL_VERSION } from '../shared/protocol.ts';
import { createAttachmentReader, createAttachmentWriter } from './attachment-store.ts';
import {
  handleAttachmentDownload,
  handleAttachmentUpload,
  handleHealth,
  handleHistory,
  type HttpContext,
} from './http.ts';
import { createJsonLogger } from './log.ts';
import { createMessageReader, createMessageWriter, type MessageWriter } from './message-store.ts';
import { createRoomKeyCheck } from './room-key.ts';
import { createTestRoomKey, makeEnvelope, openTemporaryDatabase } from './test-helpers.ts';

const ORIGIN = 'http://192.168.1.185:8081';

type Harness = { context: HttpContext; roomKey: string; logLines: string[]; writer: MessageWriter };

function createHarness(t: TestContext, options: { isConfigured?: boolean } = {}): Harness {
  const database = openTemporaryDatabase(t);
  const roomKey = createTestRoomKey();
  const logLines: string[] = [];
  const context: HttpContext = {
    isRoomKey: options.isConfigured === false ? null : createRoomKeyCheck(roomKey),
    databaseId: database.databaseId,
    messages: createMessageReader(database),
    attachments: { ...createAttachmentReader(database), ...createAttachmentWriter(database) },
    log: createJsonLogger((line) => logLines.push(line)),
    now: () => new Date('2026-10-22T09:00:00.000Z'),
  };
  t.after(() => {
    for (const line of logLines) {
      assert.equal(line.includes(roomKey), false, 'the room key must never be logged');
    }
  });
  return { context, roomKey, logLines, writer: createMessageWriter(database) };
}

function apiRequest(path: string, init: RequestInit & { roomKey?: string | null } = {}): Request {
  const { roomKey = null, headers, ...rest } = init;
  const allHeaders = new Headers(headers);
  if (roomKey !== null) {
    allHeaders.set('Authorization', `Bearer ${roomKey}`);
  }
  return new Request(`${ORIGIN}${path}`, { ...rest, headers: allHeaders });
}

function uploadRequest(attachmentId: string, roomKey: string, body: BodyInit | null, headers: HeadersInit = {}): Request {
  return apiRequest(`/api/attachments/${attachmentId}`, {
    method: 'PUT',
    roomKey,
    body,
    headers: { 'X-Chat-Uploader': 'alice', ...headers },
    ...(body instanceof ReadableStream ? { duplex: 'half' } : {}),
  });
}

async function errorCodeOf(response: Response): Promise<string> {
  const body: unknown = await response.json();
  assert.ok(typeof body === 'object' && body !== null && 'error' in body);
  const { error } = body;
  assert.ok(typeof error === 'object' && error !== null && 'code' in error && 'message' in error);
  return String(error.code);
}

function sha256Hex(bytes: Uint8Array): string {
  return createHash('sha256').update(bytes).digest('hex');
}

describe('room key on every route', () => {
  const routes = [
    { name: 'health', call: (request: Request, context: HttpContext) => handleHealth(request, context) },
    { name: 'history', call: (request: Request, context: HttpContext) => handleHistory(request, context) },
    {
      name: 'upload',
      call: (request: Request, context: HttpContext) => handleAttachmentUpload(request, randomUUID(), context),
    },
    {
      name: 'download',
      call: (request: Request, context: HttpContext) => handleAttachmentDownload(request, randomUUID(), context),
    },
  ];

  it('answers 401 without the key or with a wrong one, and logs the attempt without the key', async (t) => {
    const { context, roomKey, logLines } = createHarness(t);
    for (const { name, call } of routes) {
      for (const presented of [null, `${roomKey}x`, createTestRoomKey()]) {
        const response = await call(apiRequest('/api/messages?roomId=main', { roomKey: presented }), context);
        assert.equal(response.status, 401, name);
        assert.equal(response.headers.get('WWW-Authenticate'), 'Bearer');
        assert.equal(await errorCodeOf(response), 'unauthorized');
      }
    }
    assert.equal(logLines.length, routes.length * 3);
    assert.match(logLines[0], /"event":"http_unauthorized".*"reason":"missing_key"/);
    assert.match(logLines[1], /"reason":"wrong_key"/);
  });

  it('answers 503 on every route when no key is configured', async (t) => {
    const { context } = createHarness(t, { isConfigured: false });
    for (const { name, call } of routes) {
      const response = await call(apiRequest('/api/health', { roomKey: 'anything' }), context);
      assert.equal(response.status, 503, name);
      assert.equal(await errorCodeOf(response), 'not_configured');
    }
  });
});

describe('GET /api/health', () => {
  it('reports the protocol', async (t) => {
    const { context, roomKey } = createHarness(t);
    const response = await handleHealth(apiRequest('/api/health', { roomKey }), context);
    assert.equal(response.status, 200);
    assert.equal(response.headers.get('Cache-Control'), 'no-store');
    assert.deepEqual(await response.json(), { ok: true, protocol: PROTOCOL_VERSION });
  });
});

describe('GET /api/messages', () => {
  it('returns the newest 50 by default, in ascending order, with the database ID', async (t) => {
    const { context, roomKey, writer } = createHarness(t);
    for (let index = 0; index < 60; index += 1) {
      writer.storeMessage(makeEnvelope(), '2026-10-22T09:00:00.000Z');
    }
    const response = await handleHistory(apiRequest('/api/messages?roomId=main', { roomKey }), context);
    assert.equal(response.status, 200);
    const page = await response.json();
    assert.equal(page.databaseId, context.databaseId);
    assert.equal(page.latestSeq, 60);
    assert.equal(page.hasMore, true);
    assert.deepEqual(
      page.messages.map((message: { seq: number }) => message.seq),
      Array.from({ length: 50 }, (_, index) => index + 11),
    );
  });

  it('pages with limit, before and after', async (t) => {
    const { context, roomKey, writer } = createHarness(t);
    for (let index = 0; index < 5; index += 1) {
      writer.storeMessage(makeEnvelope(), '2026-10-22T09:00:00.000Z');
    }
    const readSeqs = async (query: string) => {
      const response = await handleHistory(apiRequest(`/api/messages?roomId=main&${query}`, { roomKey }), context);
      const page = await response.json();
      return { seqs: page.messages.map((message: { seq: number }) => message.seq), hasMore: page.hasMore };
    };
    assert.deepEqual(await readSeqs('limit=2'), { seqs: [4, 5], hasMore: true });
    assert.deepEqual(await readSeqs('limit=2&before=4'), { seqs: [2, 3], hasMore: true });
    assert.deepEqual(await readSeqs('after=3&limit=200'), { seqs: [4, 5], hasMore: false });
    assert.deepEqual(await readSeqs('after=0&limit=1'), { seqs: [1], hasMore: true });
  });

  it('answers 400 to malformed queries', async (t) => {
    const { context, roomKey } = createHarness(t);
    const queries = [
      '',
      'roomId=lobby',
      'roomId=main&roomId=main',
      'roomId=main&limit=0',
      'roomId=main&limit=201',
      'roomId=main&limit=ten',
      'roomId=main&before=3&after=1',
      'roomId=main&before=-1',
      'roomId=main&after=1.5',
      'roomId=main&after=99999999999999999999',
    ];
    for (const query of queries) {
      const response = await handleHistory(apiRequest(`/api/messages?${query}`, { roomKey }), context);
      assert.equal(response.status, 400, query);
      assert.equal(await errorCodeOf(response), 'invalid_query');
    }
  });
});

describe('PUT /api/attachments/{id}', () => {
  it('stores new bytes with 201, accepts the same bytes again with 200, and refuses different bytes with 409', async (t) => {
    const { context, roomKey, logLines } = createHarness(t);
    const attachmentId = randomUUID();
    const bytes = new TextEncoder().encode('%PDF-1.7 the plan');

    const created = await handleAttachmentUpload(uploadRequest(attachmentId, roomKey, bytes), attachmentId, context);
    assert.equal(created.status, 201);
    assert.deepEqual(await created.json(), { id: attachmentId, byteSize: bytes.byteLength, sha256: sha256Hex(bytes) });
    assert.match(logLines.at(-1) ?? '', /"event":"attachment_stored".*"uploaderId":"alice"/);

    const repeated = await handleAttachmentUpload(uploadRequest(attachmentId, roomKey, bytes), attachmentId, context);
    assert.equal(repeated.status, 200);
    assert.deepEqual(await repeated.json(), { id: attachmentId, byteSize: bytes.byteLength, sha256: sha256Hex(bytes) });

    const different = new TextEncoder().encode('%PDF-1.7 another plan');
    const conflict = await handleAttachmentUpload(uploadRequest(attachmentId, roomKey, different), attachmentId, context);
    assert.equal(conflict.status, 409);
    assert.equal(await errorCodeOf(conflict), 'attachment_conflict');
  });

  it('refuses a declared size over the limit with 413', async (t) => {
    const { context, roomKey } = createHarness(t);
    const attachmentId = randomUUID();
    const request = uploadRequest(attachmentId, roomKey, new Uint8Array(10), {
      'Content-Length': String(CHAT_LIMITS.maxAttachmentBytes + 1),
    });
    const response = await handleAttachmentUpload(request, attachmentId, context);
    assert.equal(response.status, 413);
    assert.equal(await errorCodeOf(response), 'attachment_too_large');
  });

  it('counts bytes while reading, so a body without Content-Length cannot pass the limit', async (t) => {
    const { context, roomKey } = createHarness(t);
    const attachmentId = randomUUID();
    const chunk = new Uint8Array(1024 * 1024);
    let chunksLeft = Math.ceil(CHAT_LIMITS.maxAttachmentBytes / chunk.byteLength) + 1;
    const body = new ReadableStream<Uint8Array>({
      pull(controller) {
        if (chunksLeft === 0) {
          controller.close();
          return;
        }
        chunksLeft -= 1;
        controller.enqueue(chunk);
      },
    });
    const response = await handleAttachmentUpload(uploadRequest(attachmentId, roomKey, body), attachmentId, context);
    assert.equal(response.status, 413);
    assert.equal(context.attachments.findAttachment(attachmentId), null);
  });

  it('accepts exactly the limit', async (t) => {
    const { context, roomKey } = createHarness(t);
    const attachmentId = randomUUID();
    const bytes = new Uint8Array(CHAT_LIMITS.maxAttachmentBytes);
    const response = await handleAttachmentUpload(uploadRequest(attachmentId, roomKey, bytes), attachmentId, context);
    assert.equal(response.status, 201);
  });

  it('answers 400 to a bad ID, uploader or Content-Length, and to a short body', async (t) => {
    const { context, roomKey } = createHarness(t);
    const bytes = new Uint8Array([1, 2, 3]);
    const attachmentId = randomUUID();
    const cases: { id: string; headers: Record<string, string>; code: string }[] = [
      { id: 'not-a-uuid', headers: {}, code: 'invalid_attachment_id' },
      { id: attachmentId.toUpperCase(), headers: {}, code: 'invalid_attachment_id' },
      { id: attachmentId, headers: { 'X-Chat-Uploader': 'carol' }, code: 'unknown_uploader' },
      { id: attachmentId, headers: { 'Content-Length': 'three' }, code: 'invalid_content_length' },
      { id: attachmentId, headers: { 'Content-Length': '4' }, code: 'incomplete_upload' },
    ];
    for (const { id, headers, code } of cases) {
      const response = await handleAttachmentUpload(uploadRequest(id, roomKey, bytes, headers), id, context);
      assert.equal(response.status, 400, code);
      assert.equal(await errorCodeOf(response), code);
    }
    const withoutUploader = apiRequest(`/api/attachments/${attachmentId}`, { method: 'PUT', roomKey, body: bytes });
    const response = await handleAttachmentUpload(withoutUploader, attachmentId, context);
    assert.equal(await errorCodeOf(response), 'unknown_uploader');
    assert.equal(context.attachments.findAttachment(attachmentId), null);
  });
});

describe('GET /api/attachments/{id}', () => {
  it('returns the bytes as a download that browsers will not render', async (t) => {
    const { context, roomKey } = createHarness(t);
    const attachmentId = randomUUID();
    const bytes = new TextEncoder().encode('<html><script>alert(1)</script></html>');
    await handleAttachmentUpload(uploadRequest(attachmentId, roomKey, bytes), attachmentId, context);

    const response = await handleAttachmentDownload(
      apiRequest(`/api/attachments/${attachmentId}`, { roomKey }),
      attachmentId,
      context,
    );
    assert.equal(response.status, 200);
    assert.equal(response.headers.get('Content-Type'), 'application/octet-stream');
    assert.equal(response.headers.get('Content-Disposition'), 'attachment');
    assert.equal(response.headers.get('X-Content-Type-Options'), 'nosniff');
    assert.equal(response.headers.get('Cache-Control'), 'private, max-age=31536000, immutable');
    assert.deepEqual(new Uint8Array(await response.arrayBuffer()), bytes);
  });

  it('answers 404 for unknown and malformed IDs', async (t) => {
    const { context, roomKey } = createHarness(t);
    for (const attachmentId of [randomUUID(), 'not-a-uuid']) {
      const response = await handleAttachmentDownload(
        apiRequest(`/api/attachments/${attachmentId}`, { roomKey }),
        attachmentId,
        context,
      );
      assert.equal(response.status, 404);
      assert.equal(await errorCodeOf(response), 'not_found');
    }
  });
});
