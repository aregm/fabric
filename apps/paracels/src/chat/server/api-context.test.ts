import assert from 'node:assert/strict';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { after, before, describe, it } from 'node:test';

import { getApiContext, runApiHandler } from './api-context.ts';
import type { ChatDatabase } from './database.ts';
import { createTestRoomKey } from './test-helpers.ts';

describe('API context', () => {
  const directory = mkdtempSync(join(tmpdir(), 'paracels-chat-'));

  before(() => {
    process.env.CHAT_DB_PATH = join(directory, 'chat.sqlite');
    delete process.env.EXPO_PUBLIC_CHAT_ROOM_KEY;
  });

  after(() => {
    const holder: typeof globalThis & { __paracelsChatDatabases?: Map<string, ChatDatabase> } = globalThis;
    for (const database of holder.__paracelsChatDatabases?.values() ?? []) {
      database.connection.close();
    }
    rmSync(directory, { recursive: true, force: true });
  });

  it('fails closed without a room key, and keeps one database connection per process', () => {
    const withoutKey = getApiContext();
    assert.equal(withoutKey.isRoomKey, null);

    const roomKey = createTestRoomKey();
    process.env.EXPO_PUBLIC_CHAT_ROOM_KEY = roomKey;
    const withKey = getApiContext();
    assert.notEqual(withKey, withoutKey);
    assert.equal(withKey.isRoomKey?.(roomKey), true);
    assert.equal(withKey.databaseId, withoutKey.databaseId);
    assert.equal(getApiContext(), withKey);
  });

  it('turns an unexpected error into a 500 that reveals nothing about it', async () => {
    const response = await runApiHandler(async () => {
      throw new Error('details that must stay on the server');
    });
    assert.equal(response.status, 500);
    const body = await response.text();
    assert.equal(body.includes('details'), false);
    assert.deepEqual(JSON.parse(body), {
      error: { code: 'internal_error', message: 'The chat server hit an unexpected error.' },
    });
  });
});
