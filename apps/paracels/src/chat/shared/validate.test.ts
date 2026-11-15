import assert from 'node:assert/strict';
import { describe, it } from 'node:test';

import { CHAT_LIMITS } from './limits.ts';
import { PROTOCOL_VERSION } from './protocol.ts';
import {
  isTimestamp,
  isUuid,
  parseClientFrame,
  parseServerFrame,
  validateEnvelope,
  validatePlainPayload,
  validateStoredMessage,
} from './validate.ts';

const MESSAGE_ID = '0b6f6a7e-3c1e-4c38-9a0e-2a4e7f1d9b21';
const ATTACHMENT_ID = '5d0e2c41-8a7b-4f0e-b3d2-91c4a6e7f830';
const DATABASE_ID = '7c1d3e5f-9a2b-4c6d-8e0f-1a2b3c4d5e6f';
const SHA256 = 'ab'.repeat(32);
const TIME = '2026-10-21T10:42:05.123Z';

function envelope(overrides: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    id: MESSAGE_ID,
    roomId: 'main',
    senderId: 'alice',
    clientCreatedAt: TIME,
    attachmentId: null,
    payload: { format: 'plain/1', text: 'Hello', attachment: null },
    ...overrides,
  };
}

function attachment(overrides: Record<string, unknown> = {}): Record<string, unknown> {
  return { fileName: 'plan.pdf', mimeType: 'application/pdf', byteSize: 482113, sha256: SHA256, ...overrides };
}

describe('isUuid', () => {
  it('accepts a lowercase UUID v4', () => {
    assert.equal(isUuid(MESSAGE_ID), true);
  });

  it('rejects uppercase, other versions and other shapes', () => {
    const values = [
      MESSAGE_ID.toUpperCase(),
      '0b6f6a7e-3c1e-1c38-9a0e-2a4e7f1d9b21',
      '0b6f6a7e-3c1e-4c38-7a0e-2a4e7f1d9b21',
      MESSAGE_ID.replaceAll('-', ''),
      '',
      42,
      null,
    ];
    for (const value of values) {
      assert.equal(isUuid(value), false, String(value));
    }
  });
});

describe('isTimestamp', () => {
  it('accepts the form toISOString writes', () => {
    assert.equal(isTimestamp(TIME), true);
    assert.equal(isTimestamp(new Date().toISOString()), true);
  });

  it('rejects other forms and impossible dates', () => {
    const values = [
      '2026-10-21T10:42:05Z',
      '2026-10-21T10:42:05.123+02:00',
      '2026-10-21 10:42:05.123Z',
      '2026-02-30T00:00:00.000Z',
      '2026-10-21T24:00:00.000Z',
      1792577000000,
    ];
    for (const value of values) {
      assert.equal(isTimestamp(value), false, String(value));
    }
  });
});

describe('validateEnvelope', () => {
  it('accepts an envelope and keeps its payload exactly as sent', () => {
    const payload = { format: 'plain/1', text: 'Hi', attachment: null };
    const result = validateEnvelope(envelope({ payload }));
    assert.ok(result.ok);
    assert.equal(JSON.stringify(result.value.payload), JSON.stringify(payload));
    assert.equal(result.value.attachmentId, null);
  });

  it('accepts an attachment ID', () => {
    const result = validateEnvelope(envelope({ attachmentId: ATTACHMENT_ID }));
    assert.ok(result.ok);
    assert.equal(result.value.attachmentId, ATTACHMENT_ID);
  });

  it('leaves payload fields to the policy module', () => {
    const result = validateEnvelope(envelope({ payload: { format: 'e2ee/1', ciphertext: 'opaque' } }));
    assert.ok(result.ok);
    assert.equal(result.value.payload.format, 'e2ee/1');
  });

  it('rejects unexpected or missing fields and malformed values', () => {
    const withoutAttachmentId = envelope();
    delete withoutAttachmentId.attachmentId;
    const values = [
      envelope({ extra: true }),
      withoutAttachmentId,
      envelope({ id: 'not-a-uuid' }),
      envelope({ roomId: '' }),
      envelope({ senderId: 'x'.repeat(65) }),
      envelope({ clientCreatedAt: '2026-10-21T10:42:05Z' }),
      envelope({ attachmentId: 'not-a-uuid' }),
      envelope({ payload: 'Hello' }),
      envelope({ payload: { text: 'no format' } }),
      envelope({ payload: [] }),
      [],
      null,
    ];
    for (const value of values) {
      assert.equal(validateEnvelope(value).ok, false, JSON.stringify(value));
    }
  });
});

describe('validatePlainPayload', () => {
  it('accepts text with or without an attachment', () => {
    assert.deepEqual(validatePlainPayload({ format: 'plain/1', text: 'Hello', attachment: null }), {
      ok: true,
      value: { format: 'plain/1', text: 'Hello', attachment: null },
    });
    const withAttachment = validatePlainPayload({ format: 'plain/1', text: '', attachment: attachment() });
    assert.ok(withAttachment.ok);
    assert.deepEqual(withAttachment.value.attachment, attachment());
  });

  it('rejects other formats, missing fields and malformed attachments', () => {
    const values = [
      { format: 'plain/2', text: 'Hello', attachment: null },
      { format: 'plain/1', attachment: null },
      { format: 'plain/1', text: 'Hello' },
      { format: 'plain/1', text: 'Hello', attachment: null, extra: 1 },
      { format: 'plain/1', text: 7, attachment: null },
      { format: 'plain/1', text: '', attachment: attachment({ extra: 1 }) },
      { format: 'plain/1', text: '', attachment: attachment({ sha256: SHA256.toUpperCase() }) },
      { format: 'plain/1', text: '', attachment: attachment({ byteSize: -1 }) },
      { format: 'plain/1', text: '', attachment: attachment({ byteSize: 1.5 }) },
      { format: 'plain/1', text: '', attachment: attachment({ fileName: '' }) },
      { format: 'plain/1', text: '', attachment: attachment({ fileName: 'x'.repeat(256) }) },
      { format: 'plain/1', text: '', attachment: attachment({ mimeType: 'pdf' }) },
      { format: 'plain/1', text: '', attachment: attachment({ mimeType: 'text/plain; charset=utf-8' }) },
      { format: 'plain/1', text: '', attachment: 'plan.pdf' },
    ];
    for (const value of values) {
      assert.equal(validatePlainPayload(value).ok, false, JSON.stringify(value));
    }
  });
});

describe('validateStoredMessage', () => {
  it('accepts an envelope with seq and serverReceivedAt, ignoring extra fields', () => {
    const result = validateStoredMessage({ ...envelope(), seq: 42, serverReceivedAt: TIME, extra: 1 });
    assert.ok(result.ok);
    assert.equal(result.value.seq, 42);
    assert.equal('extra' in result.value, false);
  });

  it('rejects a missing or invalid seq or server time', () => {
    const values = [
      { ...envelope(), seq: 0, serverReceivedAt: TIME },
      { ...envelope(), seq: 1.5, serverReceivedAt: TIME },
      { ...envelope(), seq: 1 },
      { ...envelope({ id: 'x' }), seq: 1, serverReceivedAt: TIME },
    ];
    for (const value of values) {
      assert.equal(validateStoredMessage(value).ok, false, JSON.stringify(value));
    }
  });
});

describe('parseClientFrame', () => {
  it('parses hello, send and ping', () => {
    assert.deepEqual(parseClientFrame(JSON.stringify({ type: 'hello', protocol: PROTOCOL_VERSION, roomKey: 'k' })), {
      ok: true,
      frame: { type: 'hello', protocol: PROTOCOL_VERSION, roomKey: 'k' },
    });
    assert.deepEqual(parseClientFrame('{"type":"ping"}'), { ok: true, frame: { type: 'ping' } });
    const send = parseClientFrame(JSON.stringify({ type: 'send', message: envelope() }));
    assert.ok(send.ok && send.frame.type === 'send');
    assert.equal(send.frame.message.id, MESSAGE_ID);
  });

  it('calls anything unusable a bad frame', () => {
    const frames = [
      'not json',
      '[]',
      '"hello"',
      '{"protocol":"paracels-chat/1"}',
      '{"type":"subscribe"}',
      '{"type":"hello","protocol":"paracels-chat/1"}',
      '{"type":"hello","protocol":"paracels-chat/1","roomKey":"k","user":"alice"}',
      '{"type":"ping","at":1}',
      JSON.stringify({ type: 'send' }),
      JSON.stringify({ type: 'send', message: envelope({ id: 'not-a-uuid' }) }),
      JSON.stringify({ type: 'send', message: envelope(), extra: 1 }),
    ];
    for (const frame of frames) {
      const result = parseClientFrame(frame);
      assert.equal(!result.ok && result.error, 'bad_frame', frame);
    }
  });

  it('reports an invalid message by its ID, so the server can reject just that message', () => {
    const result = parseClientFrame(JSON.stringify({ type: 'send', message: envelope({ clientCreatedAt: 'yesterday' }) }));
    assert.ok(!result.ok && result.error === 'invalid_message');
    assert.equal(result.messageId, MESSAGE_ID);
  });
});

describe('parseServerFrame', () => {
  it('parses every server frame type', () => {
    const frames = [
      { type: 'welcome', protocol: PROTOCOL_VERSION, databaseId: DATABASE_ID, latestSeq: 0, serverTime: TIME, limits: CHAT_LIMITS },
      { type: 'ack', id: MESSAGE_ID, seq: 42, serverReceivedAt: TIME, duplicate: false },
      { type: 'reject', id: MESSAGE_ID, code: 'unknown_sender', detail: 'carol is not a demo user' },
      { type: 'message', message: { ...envelope(), seq: 42, serverReceivedAt: TIME } },
      { type: 'pong' },
      { type: 'error', code: 'unauthorized', detail: 'wrong room key' },
      { type: 'error', code: 'internal_error', detail: 'The server hit an unexpected error.' },
    ];
    for (const frame of frames) {
      assert.deepEqual(parseServerFrame(JSON.stringify(frame)), { kind: 'frame', frame });
    }
  });

  it('marks unknown frame types for the app to ignore', () => {
    assert.deepEqual(parseServerFrame('{"type":"typing","userId":"bob"}'), { kind: 'unknown' });
  });

  it('drops extra fields of known frames', () => {
    const result = parseServerFrame(JSON.stringify({ type: 'pong', at: 1 }));
    assert.deepEqual(result, { kind: 'frame', frame: { type: 'pong' } });
  });

  it('reports malformed known frames as invalid', () => {
    const frames = [
      'not json',
      '{"type":"ack","id":"x","seq":1,"serverReceivedAt":"2026-10-21T10:42:05.123Z","duplicate":false}',
      JSON.stringify({ type: 'ack', id: MESSAGE_ID, seq: 0, serverReceivedAt: TIME, duplicate: false }),
      JSON.stringify({ type: 'reject', id: MESSAGE_ID, code: 'too_rude', detail: '' }),
      JSON.stringify({ type: 'error', code: 'teapot', detail: '' }),
      JSON.stringify({ type: 'message', message: envelope() }),
      JSON.stringify({ type: 'welcome', protocol: PROTOCOL_VERSION, databaseId: DATABASE_ID, latestSeq: 0, serverTime: TIME }),
    ];
    for (const frame of frames) {
      assert.equal(parseServerFrame(frame).kind, 'invalid', frame);
    }
  });
});
