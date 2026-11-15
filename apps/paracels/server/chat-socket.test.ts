import assert from 'node:assert/strict';
import { createHash, randomUUID } from 'node:crypto';
import { once } from 'node:events';
import { describe, it, type TestContext } from 'node:test';

import { WebSocket, type RawData } from 'ws';

import { CHAT_LIMITS } from '../src/chat/shared/limits.ts';
import {
  CLOSE_CODES,
  PROTOCOL_VERSION,
  SOCKET_PATH,
  type ServerFrame,
} from '../src/chat/shared/protocol.ts';
import { parseServerFrame } from '../src/chat/shared/validate.ts';
import { createAttachmentWriter } from '../src/chat/server/attachment-store.ts';
import type { ChatDatabase } from '../src/chat/server/database.ts';
import { createJsonLogger } from '../src/chat/server/log.ts';
import { createRoomKeyCheck } from '../src/chat/server/room-key.ts';
import { createTestRoomKey, makeEnvelope, makePlainPayload, openTemporaryDatabase } from '../src/chat/server/test-helpers.ts';
import { startChatSocketServer, type ChatSocketServer } from './chat-socket.ts';

const TEXT = 'distinctive-text-xyz';

type FrameQueue = {
  /** The next frame, or a rejection when the socket closes or the wait expires. */
  next: (timeoutMs?: number) => Promise<ServerFrame>;
};

type Client = {
  socket: WebSocket;
  queue: FrameQueue;
  opened: Promise<void>;
  closed: Promise<number>;
};

type Running = {
  port: number;
  roomKey: string;
  logLines: string[];
  server: ChatSocketServer;
  database: ChatDatabase;
};

function listen(t: TestContext, options: { helloTimeoutMs?: number; pingIntervalMs?: number } = {}): Promise<Running> {
  const database = openTemporaryDatabase(t);
  const roomKey = createTestRoomKey();
  const logLines: string[] = [];
  const started = startChatSocketServer({
    database,
    isRoomKey: createRoomKeyCheck(roomKey),
    log: createJsonLogger((line) => logLines.push(line)),
    port: 0,
    host: '127.0.0.1',
    now: () => new Date('2026-10-22T09:00:00.000Z'),
    helloTimeoutMs: options.helloTimeoutMs,
    // Stay quiet unless a test is checking dead connections.
    pingIntervalMs: options.pingIntervalMs ?? 60_000,
  });
  return started.then((server) => {
    t.after(() => server.close());
    t.after(() => {
      const output = logLines.join('\n');
      assert.equal(output.includes(roomKey), false, 'the room key must never be logged');
      assert.equal(output.includes(TEXT), false, 'message text must never be logged');
    });
    return { port: server.port, roomKey, logLines, server, database };
  });
}

function connect(port: number, options: { autoPong?: boolean } = {}): Client {
  const socket = new WebSocket(`ws://127.0.0.1:${port}${SOCKET_PATH}`, { autoPong: options.autoPong });
  const tracked = track(socket);
  return {
    socket,
    ...tracked,
    opened: once(socket, 'open').then(() => undefined),
  };
}

function track(socket: WebSocket): { queue: FrameQueue; closed: Promise<number> } {
  const pending: ServerFrame[] = [];
  const waiters: ((outcome: { ok: true; frame: ServerFrame } | { ok: false; reason: string }) => void)[] = [];
  const deliver = (outcome: { ok: true; frame: ServerFrame } | { ok: false; reason: string }): void => {
    const waiter = waiters.shift();
    if (waiter !== undefined) {
      waiter(outcome);
    } else if (outcome.ok) {
      pending.push(outcome.frame);
    }
  };
  socket.on('message', (data: RawData, isBinary: boolean) => {
    if (isBinary) {
      deliver({ ok: false, reason: 'binary frame' });
      return;
    }
    const text = typeof data === 'string' || Buffer.isBuffer(data) ? data.toString('utf8') : 'unsupported frame';
    const parsed = parseServerFrame(text);
    deliver(parsed.kind === 'frame' ? { ok: true, frame: parsed.frame } : { ok: false, reason: parsed.kind });
  });
  socket.on('error', () => undefined);
  const closed = new Promise<number>((resolve) => {
    socket.once('close', (code: number) => {
      deliver({ ok: false, reason: `closed ${code}` });
      resolve(code);
    });
  });
  return {
    closed,
    queue: {
      next(timeoutMs = 2000) {
        const existing = pending.shift();
        if (existing !== undefined) {
          return Promise.resolve(existing);
        }
        return new Promise((resolve, reject) => {
          const timer = setTimeout(() => reject(new Error('timed out waiting for a frame')), timeoutMs);
          waiters.push((outcome) => {
            clearTimeout(timer);
            if (outcome.ok) {
              resolve(outcome.frame);
            } else {
              reject(new Error(outcome.reason));
            }
          });
        });
      },
    },
  };
}

async function hello(client: Client, roomKey: string, protocol = PROTOCOL_VERSION): Promise<ServerFrame> {
  await client.opened;
  client.socket.send(JSON.stringify({ type: 'hello', protocol, roomKey }));
  return client.queue.next();
}

function sendMessage(client: Client, message: unknown): void {
  client.socket.send(JSON.stringify({ type: 'send', message }));
}

describe('chat socket server', () => {
  it('welcomes a client and tells it the database and the limits', async (t) => {
    const running = await listen(t);
    const client = connect(running.port);
    t.after(() => client.socket.close());
    const welcome = await hello(client, running.roomKey);
    assert.deepEqual(welcome, {
      type: 'welcome',
      protocol: PROTOCOL_VERSION,
      databaseId: running.database.databaseId,
      latestSeq: 0,
      serverTime: '2026-10-22T09:00:00.000Z',
      limits: CHAT_LIMITS,
    });
    assert.deepEqual(
      running.logLines.map((line) => JSON.parse(line).event),
      ['connection_opened'],
    );
  });

  it('relays a new message to the other client and only acknowledges a repeat', async (t) => {
    const running = await listen(t);
    const alice = connect(running.port);
    const bob = connect(running.port);
    t.after(() => alice.socket.close());
    t.after(() => bob.socket.close());
    await hello(alice, running.roomKey);
    await hello(bob, running.roomKey);

    const envelope = makeEnvelope({ payload: makePlainPayload(TEXT), senderId: 'alice' });
    sendMessage(alice, envelope);
    const ack = await alice.queue.next();
    const relayed = await bob.queue.next();
    assert.ok(ack.type === 'ack' && relayed.type === 'message');
    assert.equal(ack.duplicate, false);
    assert.equal(ack.seq, 1);
    assert.equal(ack.serverReceivedAt, '2026-10-22T09:00:00.000Z');
    assert.deepEqual(relayed.message, { ...envelope, seq: 1, serverReceivedAt: ack.serverReceivedAt });

    sendMessage(alice, envelope);
    const repeat = await alice.queue.next();
    assert.deepEqual(repeat, { ...ack, duplicate: true });
    await assert.rejects(bob.queue.next(150), /timed out/);

    const stored = running.logLines.filter((line) => line.includes('"message_stored"'));
    assert.equal(stored.length, 2);
    assert.match(stored[0], new RegExp(`"seq":1.*"senderId":"alice".*"textLength":${TEXT.length}.*"hasAttachment":false.*"duplicate":false`));
    assert.match(stored[1], /"duplicate":true/);
  });

  it('rejects a conflicting resend and an unknown sender without dropping the connection', async (t) => {
    const running = await listen(t);
    const client = connect(running.port);
    t.after(() => client.socket.close());
    await hello(client, running.roomKey);
    const envelope = makeEnvelope({ payload: makePlainPayload('Hello') });
    sendMessage(client, envelope);
    assert.equal((await client.queue.next()).type, 'ack');

    sendMessage(client, { ...envelope, payload: makePlainPayload('Edited') });
    const conflict = await client.queue.next();
    assert.deepEqual(conflict, {
      type: 'reject',
      id: envelope.id,
      code: 'id_conflict',
      detail: 'This message ID is already stored with different content.',
    });

    const rejected = makeEnvelope({ senderId: 'carol', payload: makePlainPayload('Hi') });
    sendMessage(client, rejected);
    const unknown = await client.queue.next();
    assert.ok(unknown.type === 'reject' && unknown.code === 'unknown_sender');

    sendMessage(client, makeEnvelope({ payload: makePlainPayload('Still here') }));
    const ack = await client.queue.next();
    // A conflicting insert is refused, but SQLite may still consume that sequence number.
    assert.ok(ack.type === 'ack' && ack.seq > 1);
    const count = running.database.connection.prepare('SELECT COUNT(*) AS count FROM messages').get();
    assert.equal(count?.count, 2);
  });

  it('rejects a message whose attachment was not uploaded, then accepts it after the upload', async (t) => {
    const running = await listen(t);
    const attachmentId = randomUUID();
    const bytes = new TextEncoder().encode('hello');
    const sha256 = createHash('sha256').update(bytes).digest('hex');
    const envelope = makeEnvelope({
      attachmentId,
      payload: makePlainPayload('', { fileName: 'note.txt', mimeType: 'text/plain', byteSize: bytes.length, sha256 }),
    });
    const client = connect(running.port);
    t.after(() => client.socket.close());
    await hello(client, running.roomKey);

    sendMessage(client, envelope);
    const missing = await client.queue.next();
    assert.ok(missing.type === 'reject' && missing.code === 'attachment_missing');

    // The upload is what the API route would have stored. This server only reads it.
    createAttachmentWriter(running.database).storeAttachment({
      id: attachmentId,
      uploaderId: 'alice',
      bytes,
      createdAt: '2026-10-22T09:00:00.000Z',
    });
    sendMessage(client, envelope);
    const ack = await client.queue.next();
    assert.ok(ack.type === 'ack' && ack.duplicate === false);
    assert.equal(ack.seq, 1);
  });

  it('answers a ping and reports a malformed message without closing', async (t) => {
    const running = await listen(t);
    const client = connect(running.port);
    t.after(() => client.socket.close());
    await hello(client, running.roomKey);
    client.socket.send(JSON.stringify({ type: 'ping' }));
    assert.deepEqual(await client.queue.next(), { type: 'pong' });

    const envelope = makeEnvelope();
    sendMessage(client, { ...envelope, clientCreatedAt: 'yesterday' });
    const rejected = await client.queue.next();
    assert.ok(rejected.type === 'reject' && rejected.id === envelope.id && rejected.code === 'invalid_message');
    assert.equal(client.socket.readyState, WebSocket.OPEN);
  });

  it('closes connections that fail hello', async (t) => {
    const running = await listen(t, { helloTimeoutMs: 80 });
    const cases: { name: string; send: (client: Client) => void; code: string; closeCode: number }[] = [
      {
        name: 'wrong key',
        send: (client) => client.socket.send(JSON.stringify({ type: 'hello', protocol: PROTOCOL_VERSION, roomKey: 'nope' })),
        code: 'unauthorized',
        closeCode: CLOSE_CODES.unauthorized,
      },
      {
        name: 'wrong protocol',
        send: (client) =>
          client.socket.send(JSON.stringify({ type: 'hello', protocol: 'paracels-chat/2', roomKey: running.roomKey })),
        code: 'unsupported_protocol',
        closeCode: CLOSE_CODES.badFrame,
      },
      {
        name: 'message before hello',
        send: (client) => sendMessage(client, makeEnvelope()),
        code: 'not_ready',
        closeCode: CLOSE_CODES.badFrame,
      },
    ];
    for (const { name, send, code, closeCode } of cases) {
      const client = connect(running.port);
      await client.opened;
      send(client);
      const error = await client.queue.next();
      assert.ok(error.type === 'error' && error.code === code, name);
      assert.equal(await client.closed, closeCode, name);
    }

    const silent = connect(running.port);
    await silent.opened;
    const timeout = await silent.queue.next();
    assert.ok(timeout.type === 'error' && timeout.code === 'hello_timeout');
    assert.equal(await silent.closed, CLOSE_CODES.helloTimeout);
    assert.match(running.logLines.join('\n'), /"event":"hello_refused".*"code":"unauthorized"/);
  });

  it('closes a bad frame after hello and keeps serving the next client', async (t) => {
    const running = await listen(t);
    const client = connect(running.port);
    await hello(client, running.roomKey);
    client.socket.send('{');
    const error = await client.queue.next();
    assert.ok(error.type === 'error' && error.code === 'bad_frame');
    assert.equal(await client.closed, CLOSE_CODES.badFrame);

    const next = connect(running.port);
    t.after(() => next.socket.close());
    assert.equal((await hello(next, running.roomKey)).type, 'welcome');
  });

  it('survives a frame over the size limit and keeps serving', async (t) => {
    const running = await listen(t);
    const client = connect(running.port);
    await client.opened;
    client.socket.send(Buffer.alloc(CHAT_LIMITS.maxFrameBytes + 1024));
    assert.equal(await client.closed, CLOSE_CODES.frameTooLarge);

    const next = connect(running.port);
    t.after(() => next.socket.close());
    const welcome = await hello(next, running.roomKey);
    assert.equal(welcome.type, 'welcome');
    assert.match(running.logLines.join('\n'), /"event":"connection_error"/);
  });

  it('drops a connection that stops answering pings', async (t) => {
    const running = await listen(t, { pingIntervalMs: 80 });
    const client = connect(running.port, { autoPong: false });
    await hello(client, running.roomKey);
    assert.equal(await client.closed, 1006);
  });

  it('refuses a connection on any path other than /chat', async (t) => {
    const running = await listen(t);
    const socket = new WebSocket(`ws://127.0.0.1:${running.port}/other`);
    const status = await new Promise<number>((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error('timed out waiting for the handshake to be refused')), 2000);
      socket.on('unexpected-response', (_request, response) => {
        clearTimeout(timer);
        const statusCode = response.statusCode ?? 0;
        response.resume();
        socket.terminate();
        resolve(statusCode);
      });
      socket.on('error', () => undefined);
    });
    assert.equal(status, 400);
  });

  it('closes open connections with 1001 when it shuts down', async (t) => {
    const running = await listen(t);
    const client = connect(running.port);
    await hello(client, running.roomKey);
    const closed = client.closed;
    await running.server.close();
    assert.equal(await closed, CLOSE_CODES.serverShuttingDown);
  });
});
