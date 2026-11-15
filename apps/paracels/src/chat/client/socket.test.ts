import assert from 'node:assert/strict';
import { describe, it } from 'node:test';

import { createJsonLogger } from '../server/log.ts';
import { createRoomKeyCheck } from '../server/room-key.ts';
import { createTestRoomKey, makeEnvelope, openTemporaryDatabase } from '../server/test-helpers.ts';
import { startChatSocketServer } from '../../../server/chat-socket.ts';
import { readChatConfig } from './config.ts';
import { createChatSocket, type ChatSocketEvent } from './socket.ts';

async function waitFor(
  events: ChatSocketEvent[],
  match: (event: ChatSocketEvent) => boolean,
  label: string,
): Promise<void> {
  const started = Date.now();
  while (!events.some(match)) {
    if (Date.now() - started > 2000) {
      throw new Error(label);
    }
    await new Promise((resolve) => setTimeout(resolve, 20));
  }
}

describe('createChatSocket', () => {
  it('connects, receives welcome, sends a message and sees the ack', async (t) => {
    const database = openTemporaryDatabase(t);
    const roomKey = createTestRoomKey();
    const server = await startChatSocketServer({
      database,
      isRoomKey: createRoomKeyCheck(roomKey),
      log: createJsonLogger(() => undefined),
      port: 0,
      host: '127.0.0.1',
    });
    t.after(() => server.close());

    const config = readChatConfig(
      { EXPO_PUBLIC_CHAT_ROOM_KEY: roomKey, EXPO_PUBLIC_CHAT_SOCKET_URL: `ws://127.0.0.1:${server.port}/chat` },
      null,
    );
    const socket = createChatSocket({ config });
    t.after(() => socket.disconnect());

    const events: ChatSocketEvent[] = [];
    socket.subscribe((event) => events.push(event));
    socket.connect();
    await waitFor(events, (event) => event.type === 'frame' && event.frame.type === 'welcome', 'no welcome');
    assert.equal(socket.status(), 'connected');

    const envelope = makeEnvelope();
    assert.equal(socket.send({ type: 'send', message: envelope }), true);
    await waitFor(events, (event) => event.type === 'frame' && event.frame.type === 'ack', 'no ack');
    const ack = events.find((event) => event.type === 'frame' && event.frame.type === 'ack');
    assert.ok(ack && ack.type === 'frame' && ack.frame.type === 'ack');
    assert.equal(ack.frame.id, envelope.id);
  });
});
