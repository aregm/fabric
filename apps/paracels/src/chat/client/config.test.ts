import assert from 'node:assert/strict';
import { describe, it } from 'node:test';

import { readChatConfig, reconnectDelayMs } from './config.ts';

describe('readChatConfig', () => {
  it('builds the socket URL from the bundle host', () => {
    const config = readChatConfig(
      { EXPO_PUBLIC_CHAT_ROOM_KEY: ' secret ' },
      { hostname: '192.168.1.185', origin: 'http://192.168.1.185:8081', protocol: 'http:' },
    );
    assert.equal(config.socketUrl, 'ws://192.168.1.185:8787/chat');
    assert.equal(config.socketDisplay, '192.168.1.185:8787');
    assert.equal(config.apiOrigin, 'http://192.168.1.185:8081');
    assert.equal(config.roomKey, 'secret');
  });

  it('lets EXPO_PUBLIC_CHAT_SOCKET_URL replace the socket address', () => {
    const config = readChatConfig(
      { EXPO_PUBLIC_CHAT_SOCKET_URL: 'ws://example.test:9/chat' },
      { hostname: 'localhost', origin: 'http://localhost:8081', protocol: 'http:' },
    );
    assert.equal(config.socketUrl, 'ws://example.test:9/chat');
    assert.equal(config.roomKey, null);
  });
});

describe('reconnectDelayMs', () => {
  it('starts at 1 second and doubles up to 30, with jitter', () => {
    assert.equal(reconnectDelayMs(0, 0.5), 1000);
    assert.equal(reconnectDelayMs(1, 0.5), 2000);
    assert.equal(reconnectDelayMs(10, 0.5), 30000);
    assert.ok(reconnectDelayMs(0, 0) < 1000);
    assert.ok(reconnectDelayMs(0, 1) > 1000);
  });
});
