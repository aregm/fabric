import assert from 'node:assert/strict';
import { describe, it } from 'node:test';

import { readRoomKey, resolveDatabasePath, resolveSocketPort } from './config.ts';
import { createJsonLogger } from './log.ts';

describe('server configuration', () => {
  it('puts the database in .local/ unless CHAT_DB_PATH says otherwise', () => {
    assert.equal(resolveDatabasePath({}, '/work/apps/paracels'), '/work/apps/paracels/.local/chat.sqlite');
    assert.equal(resolveDatabasePath({ CHAT_DB_PATH: ' ' }, '/work'), '/work/.local/chat.sqlite');
    assert.equal(resolveDatabasePath({ CHAT_DB_PATH: 'data/test.sqlite' }, '/work'), '/work/data/test.sqlite');
    assert.equal(resolveDatabasePath({ CHAT_DB_PATH: '/tmp/chat.sqlite' }, '/work'), '/tmp/chat.sqlite');
  });

  it('reads the room key, treating a blank one as missing', () => {
    assert.equal(readRoomKey({ EXPO_PUBLIC_CHAT_ROOM_KEY: ' key-1 ' }), 'key-1');
    assert.equal(readRoomKey({ EXPO_PUBLIC_CHAT_ROOM_KEY: '  ' }), null);
    assert.equal(readRoomKey({}), null);
  });

  it('uses port 8787 unless CHAT_SOCKET_PORT names a valid port', () => {
    assert.deepEqual(resolveSocketPort({}), { ok: true, value: 8787 });
    assert.deepEqual(resolveSocketPort({ CHAT_SOCKET_PORT: '9000' }), { ok: true, value: 9000 });
    for (const value of ['0', '65536', 'abc', '80.5', '-1', '123456']) {
      assert.equal(resolveSocketPort({ CHAT_SOCKET_PORT: value }).ok, false, value);
    }
  });
});

describe('createJsonLogger', () => {
  it('writes one JSON line per event, time first', () => {
    const lines: string[] = [];
    const log = createJsonLogger(
      (line) => lines.push(line),
      () => new Date('2026-10-22T09:00:00.000Z'),
    );
    log('message_stored', { seq: 1, hasAttachment: false });
    log('server_stopped');
    assert.deepEqual(lines, [
      '{"time":"2026-10-22T09:00:00.000Z","event":"message_stored","seq":1,"hasAttachment":false}',
      '{"time":"2026-10-22T09:00:00.000Z","event":"server_stopped"}',
    ]);
  });
});
