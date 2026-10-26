import assert from 'node:assert/strict';
import { randomUUID } from 'node:crypto';
import { describe, it } from 'node:test';

import { createAttachmentReader, createAttachmentWriter } from './attachment-store.ts';
import { openTemporaryDatabase } from './test-helpers.ts';

const CREATED_AT = '2026-10-22T09:00:00.000Z';
const HELLO_BYTES = new TextEncoder().encode('hello');
const HELLO_SHA256 = '2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824';

describe('attachment store', () => {
  it('stores bytes with their size and SHA-256, and reads them back', (t) => {
    const database = openTemporaryDatabase(t);
    const id = randomUUID();
    const outcome = createAttachmentWriter(database).storeAttachment({
      id,
      uploaderId: 'alice',
      bytes: HELLO_BYTES,
      createdAt: CREATED_AT,
    });
    assert.deepEqual(outcome, { kind: 'stored', attachment: { id, byteSize: 5, sha256: HELLO_SHA256 } });

    const reader = createAttachmentReader(database);
    assert.deepEqual(reader.findAttachment(id), { id, byteSize: 5, sha256: HELLO_SHA256 });
    assert.deepEqual(reader.readAttachmentBytes(id), HELLO_BYTES);
    const row = database.connection.prepare('SELECT uploader_id, created_at FROM attachments WHERE id = ?').get(id);
    assert.deepEqual({ ...row }, { uploader_id: 'alice', created_at: CREATED_AT });
  });

  it('accepts the same bytes again and refuses different bytes under the same ID', (t) => {
    const writer = createAttachmentWriter(openTemporaryDatabase(t));
    const id = randomUUID();
    writer.storeAttachment({ id, uploaderId: 'alice', bytes: HELLO_BYTES, createdAt: CREATED_AT });

    const again = writer.storeAttachment({ id, uploaderId: 'bob', bytes: HELLO_BYTES.slice(), createdAt: CREATED_AT });
    assert.deepEqual(again, { kind: 'already_stored', attachment: { id, byteSize: 5, sha256: HELLO_SHA256 } });
    const different = writer.storeAttachment({
      id,
      uploaderId: 'alice',
      bytes: new TextEncoder().encode('jello'),
      createdAt: CREATED_AT,
    });
    assert.deepEqual(different, { kind: 'conflict' });
  });

  it('stores empty files', (t) => {
    const database = openTemporaryDatabase(t);
    const id = randomUUID();
    createAttachmentWriter(database).storeAttachment({ id, uploaderId: 'bob', bytes: new Uint8Array(0), createdAt: CREATED_AT });
    assert.equal(createAttachmentReader(database).readAttachmentBytes(id)?.byteLength, 0);
  });

  it('returns null for an unknown ID', (t) => {
    const reader = createAttachmentReader(openTemporaryDatabase(t));
    assert.equal(reader.findAttachment(randomUUID()), null);
    assert.equal(reader.readAttachmentBytes(randomUUID()), null);
  });
});
