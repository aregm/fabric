import assert from 'node:assert/strict';
import { randomUUID } from 'node:crypto';
import { describe, it } from 'node:test';

import { CHAT_LIMITS } from '../shared/limits.ts';
import type { AttachmentSummary } from './attachment-store.ts';
import { readPlainMessageContent, reviewMessage, reviewUpload } from './policy.ts';
import { makeEnvelope, makePlainPayload } from './test-helpers.ts';

const ATTACHMENT_ID = randomUUID();
const STORED: AttachmentSummary = { id: ATTACHMENT_ID, byteSize: 482113, sha256: 'ab'.repeat(32) };
const ATTACHMENT_META = { fileName: 'plan.pdf', mimeType: 'application/pdf', byteSize: 482113, sha256: 'ab'.repeat(32) };

function findStored(attachmentId: string): AttachmentSummary | null {
  return attachmentId === ATTACHMENT_ID ? STORED : null;
}

function codeOf(decision: ReturnType<typeof reviewMessage>): string {
  return decision.isAccepted ? 'accepted' : decision.code;
}

describe('reviewMessage', () => {
  it('accepts text, and reports only its length and whether it has an attachment', () => {
    assert.deepEqual(reviewMessage(makeEnvelope({ payload: makePlainPayload('Hello') }), findStored), {
      isAccepted: true,
      summary: { textLength: 5, hasAttachment: false },
    });
  });

  it('accepts an attachment that matches its upload, with or without text', () => {
    for (const text of ['The plan is attached.', '']) {
      const envelope = makeEnvelope({ attachmentId: ATTACHMENT_ID, payload: makePlainPayload(text, ATTACHMENT_META) });
      assert.deepEqual(reviewMessage(envelope, findStored), {
        isAccepted: true,
        summary: { textLength: text.length, hasAttachment: true },
      });
    }
  });

  it('enforces the text limit of D5', () => {
    const atLimit = makeEnvelope({ payload: makePlainPayload('x'.repeat(CHAT_LIMITS.maxTextLength)) });
    const overLimit = makeEnvelope({ payload: makePlainPayload('x'.repeat(CHAT_LIMITS.maxTextLength + 1)) });
    assert.equal(codeOf(reviewMessage(atLimit, findStored)), 'accepted');
    assert.equal(codeOf(reviewMessage(overLimit, findStored)), 'invalid_message');
  });

  it('rejects unknown rooms, senders and formats, and malformed or empty payloads', () => {
    const cases = [
      { envelope: makeEnvelope({ roomId: 'lobby' }), code: 'invalid_message' },
      { envelope: makeEnvelope({ senderId: 'carol' }), code: 'unknown_sender' },
      { envelope: makeEnvelope({ payload: { format: 'e2ee/1', ciphertext: 'opaque' } }), code: 'invalid_message' },
      { envelope: makeEnvelope({ payload: { format: 'plain/1', text: 'Hi' } }), code: 'invalid_message' },
      { envelope: makeEnvelope({ payload: makePlainPayload('') }), code: 'invalid_message' },
      { envelope: makeEnvelope({ payload: makePlainPayload(' \n\t ') }), code: 'invalid_message' },
    ];
    for (const { envelope, code } of cases) {
      assert.equal(codeOf(reviewMessage(envelope, findStored)), code, JSON.stringify(envelope));
    }
  });

  it('requires attachmentId and the payload attachment to agree', () => {
    const idOnly = makeEnvelope({ attachmentId: ATTACHMENT_ID, payload: makePlainPayload('Hi') });
    const metaOnly = makeEnvelope({ payload: makePlainPayload('Hi', ATTACHMENT_META) });
    assert.equal(codeOf(reviewMessage(idOnly, findStored)), 'invalid_message');
    assert.equal(codeOf(reviewMessage(metaOnly, findStored)), 'invalid_message');
  });

  it('checks the attachment against the stored upload', () => {
    const missing = makeEnvelope({ attachmentId: randomUUID(), payload: makePlainPayload('', ATTACHMENT_META) });
    const wrongSize = makeEnvelope({
      attachmentId: ATTACHMENT_ID,
      payload: makePlainPayload('', { ...ATTACHMENT_META, byteSize: 1 }),
    });
    const wrongHash = makeEnvelope({
      attachmentId: ATTACHMENT_ID,
      payload: makePlainPayload('', { ...ATTACHMENT_META, sha256: 'cd'.repeat(32) }),
    });
    assert.equal(codeOf(reviewMessage(missing, findStored)), 'attachment_missing');
    assert.equal(codeOf(reviewMessage(wrongSize, findStored)), 'attachment_mismatch');
    assert.equal(codeOf(reviewMessage(wrongHash, findStored)), 'attachment_mismatch');
  });
});

describe('readPlainMessageContent', () => {
  it('returns the text and attachment name of a plain payload', () => {
    assert.deepEqual(readPlainMessageContent({ format: 'plain/1', text: 'Hello', attachment: null }), {
      text: 'Hello',
      attachmentFileName: null,
    });
    assert.deepEqual(
      readPlainMessageContent({ format: 'plain/1', text: '', attachment: ATTACHMENT_META }),
      { text: '', attachmentFileName: 'plan.pdf' },
    );
  });

  it('hides payloads it cannot read', () => {
    assert.equal(readPlainMessageContent({ format: 'e2ee/1', ciphertext: 'secret-ciphertext' }), null);
    assert.equal(readPlainMessageContent({ format: 'plain/1', text: 'Hello' }), null);
  });
});

describe('reviewUpload', () => {
  it('accepts a demo user within the size limit, with or without Content-Length', () => {
    const maxBytes = CHAT_LIMITS.maxAttachmentBytes;
    assert.deepEqual(reviewUpload('alice', 1000), { isAccepted: true, uploaderId: 'alice', maxBytes });
    assert.deepEqual(reviewUpload('bob', maxBytes), { isAccepted: true, uploaderId: 'bob', maxBytes });
    assert.deepEqual(reviewUpload('bob', null), { isAccepted: true, uploaderId: 'bob', maxBytes });
  });

  it('refuses unknown uploaders and declared sizes over the limit', () => {
    for (const uploader of [null, 'carol', 'Alice']) {
      const decision = reviewUpload(uploader, 10);
      assert.equal(!decision.isAccepted && decision.code, 'unknown_uploader');
    }
    const tooLarge = reviewUpload('alice', CHAT_LIMITS.maxAttachmentBytes + 1);
    assert.equal(!tooLarge.isAccepted && tooLarge.code, 'attachment_too_large');
  });
});
