import assert from 'node:assert/strict';
import { describe, it } from 'node:test';

import { createRoomKeyCheck, readBearerToken } from './room-key.ts';
import { createTestRoomKey } from './test-helpers.ts';

describe('room key', () => {
  it('accepts only the exact key', () => {
    const roomKey = createTestRoomKey();
    const isRoomKey = createRoomKeyCheck(roomKey);
    assert.equal(isRoomKey(roomKey), true);
    for (const presented of [`${roomKey}x`, roomKey.slice(1), roomKey.toUpperCase(), '', null]) {
      assert.equal(isRoomKey(presented), false);
    }
  });

  it('reads the key from a Bearer authorization header', () => {
    assert.equal(readBearerToken('Bearer abc.DEF-123_'), 'abc.DEF-123_');
    assert.equal(readBearerToken('bearer abc'), 'abc');
    assert.equal(readBearerToken('Bearer  abc '), 'abc');
    for (const header of ['Basic abc', 'Bearer', 'Bearer ', 'Bearer a b', 'abc', null]) {
      assert.equal(readBearerToken(header), null, String(header));
    }
  });
});
