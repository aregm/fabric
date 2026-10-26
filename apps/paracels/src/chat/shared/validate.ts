/**
 * Validators for everything that crosses the network (docs/paracels-chat.md,
 * sections 6 and 7). They return results instead of throwing, so each caller
 * decides how to answer bad input. Frames from the app are checked strictly;
 * frames from the server leniently, so the server can add frame types later.
 */

import type { ChatLimits } from './limits.ts';
import {
  CONNECTION_ERROR_CODES,
  PLAIN_PAYLOAD_FORMAT,
  REJECT_CODES,
  type ClientFrame,
  type ConnectionErrorCode,
  type MessageEnvelope,
  type OpaquePayload,
  type PlainAttachment,
  type PlainPayload,
  type RejectCode,
  type ServerFrame,
  type StoredMessage,
} from './protocol.ts';

/** The outcome of checking one value. */
export type Validation<T> = { ok: true; value: T } | { ok: false; detail: string };

/** The outcome of parsing one frame from the app. */
export type ClientFrameResult =
  | { ok: true; frame: ClientFrame }
  /** Unusable as a frame: the server answers with `error` and closes. */
  | { ok: false; error: 'bad_frame'; detail: string }
  /** A `send` whose message has a valid ID but invalid fields: the server rejects that message. */
  | { ok: false; error: 'invalid_message'; messageId: string; detail: string };

/** The outcome of parsing one frame from the server. */
export type ServerFrameResult =
  | { kind: 'frame'; frame: ServerFrame }
  /** A frame type this app doesn't know; the app ignores it. */
  | { kind: 'unknown' }
  | { kind: 'invalid'; detail: string };

const UUID_PATTERN = /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;
const TIMESTAMP_PATTERN = /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{3}Z$/;
const SHA256_PATTERN = /^[0-9a-f]{64}$/;
const MIME_TYPE_PATTERN = /^[a-z0-9][a-z0-9!#$&^_.+-]{0,126}\/[a-z0-9][a-z0-9!#$&^_.+-]{0,126}$/i;

/** Room IDs, sender IDs and payload formats are short tokens. */
const MAX_TOKEN_LENGTH = 64;
const MAX_FILE_NAME_LENGTH = 255;
/** Field names echoed in a `detail` are cut to this length. */
const MAX_ECHOED_KEY_LENGTH = 40;

const ENVELOPE_KEYS = ['id', 'roomId', 'senderId', 'clientCreatedAt', 'attachmentId', 'payload'];
const PLAIN_PAYLOAD_KEYS = ['format', 'text', 'attachment'];
const PLAIN_ATTACHMENT_KEYS = ['fileName', 'mimeType', 'byteSize', 'sha256'];

/**
 * Tells whether a value is a lowercase UUID v4, the form of every message and attachment ID.
 * @param value Any value.
 */
export function isUuid(value: unknown): value is string {
  return typeof value === 'string' && UUID_PATTERN.test(value);
}

/**
 * Tells whether a value is an ISO 8601 time in UTC with milliseconds, exactly as
 * `Date.prototype.toISOString()` writes it, naming a real date.
 * @param value Any value.
 */
export function isTimestamp(value: unknown): value is string {
  if (typeof value !== 'string' || !TIMESTAMP_PATTERN.test(value)) {
    return false;
  }
  const time = Date.parse(value);
  return Number.isFinite(time) && new Date(time).toISOString() === value;
}

/**
 * Tells whether a value is a SHA-256 digest in lowercase hex.
 * @param value Any value.
 */
export function isSha256Hex(value: unknown): value is string {
  return typeof value === 'string' && SHA256_PATTERN.test(value);
}

/**
 * Validates a message envelope from the app: exactly the six fields of section 6.1,
 * with the payload left opaque apart from its format tag.
 * @param value The `message` field of a `send` frame.
 */
export function validateEnvelope(value: unknown): Validation<MessageEnvelope> {
  if (!isRecord(value)) {
    return fail('message must be an object');
  }
  const extraKey = findUnexpectedKey(value, ENVELOPE_KEYS);
  if (extraKey !== undefined) {
    return fail(`message has an unexpected field: ${echoKey(extraKey)}`);
  }
  return checkEnvelopeFields(value);
}

/**
 * Validates a `plain/1` payload. The server's policy module and the app's
 * renderer call this; nothing else reads payload fields.
 * @param value A payload whose format tag is `plain/1`.
 */
export function validatePlainPayload(value: unknown): Validation<PlainPayload> {
  if (!isRecord(value)) {
    return fail('payload must be an object');
  }
  const extraKey = findUnexpectedKey(value, PLAIN_PAYLOAD_KEYS);
  if (extraKey !== undefined) {
    return fail(`payload has an unexpected field: ${echoKey(extraKey)}`);
  }
  if (value.format !== PLAIN_PAYLOAD_FORMAT) {
    return fail(`payload format must be ${PLAIN_PAYLOAD_FORMAT}`);
  }
  if (typeof value.text !== 'string') {
    return fail('payload text must be a string');
  }
  if (value.attachment === null) {
    return ok({ format: PLAIN_PAYLOAD_FORMAT, text: value.text, attachment: null });
  }
  const attachment = validatePlainAttachment(value.attachment);
  if (!attachment.ok) {
    return attachment;
  }
  return ok({ format: PLAIN_PAYLOAD_FORMAT, text: value.text, attachment: attachment.value });
}

/**
 * Validates a logged message from the server, as it arrives in a history page or
 * a `message` frame. Unknown extra fields are ignored.
 * @param value One element of `messages`, or the `message` of a `message` frame.
 */
export function validateStoredMessage(value: unknown): Validation<StoredMessage> {
  if (!isRecord(value)) {
    return fail('message must be an object');
  }
  const envelope = checkEnvelopeFields(value);
  if (!envelope.ok) {
    return envelope;
  }
  if (!isPositiveInteger(value.seq)) {
    return fail('seq must be a positive integer');
  }
  if (!isTimestamp(value.serverReceivedAt)) {
    return fail('serverReceivedAt must be an ISO 8601 UTC time with milliseconds');
  }
  return ok({ ...envelope.value, seq: value.seq, serverReceivedAt: value.serverReceivedAt });
}

/**
 * Parses one text frame from the app, strictly: unknown frame types and unexpected
 * fields are errors.
 * @param text The frame as received.
 */
export function parseClientFrame(text: string): ClientFrameResult {
  const parsed = parseJson(text);
  if (!parsed.ok || !isRecord(parsed.value) || typeof parsed.value.type !== 'string') {
    return badFrame('a frame must be a JSON object with a type');
  }
  const frame = parsed.value;
  switch (frame.type) {
    case 'hello': {
      const extraKey = findUnexpectedKey(frame, ['type', 'protocol', 'roomKey']);
      if (extraKey !== undefined) {
        return badFrame(`hello has an unexpected field: ${echoKey(extraKey)}`);
      }
      if (typeof frame.protocol !== 'string' || typeof frame.roomKey !== 'string') {
        return badFrame('hello needs a protocol and a roomKey');
      }
      return { ok: true, frame: { type: 'hello', protocol: frame.protocol, roomKey: frame.roomKey } };
    }
    case 'send': {
      const extraKey = findUnexpectedKey(frame, ['type', 'message']);
      if (extraKey !== undefined) {
        return badFrame(`send has an unexpected field: ${echoKey(extraKey)}`);
      }
      const message = frame.message;
      if (!isRecord(message) || !isUuid(message.id)) {
        return badFrame('send needs a message whose id is a lowercase UUID v4');
      }
      const envelope = validateEnvelope(message);
      if (!envelope.ok) {
        return { ok: false, error: 'invalid_message', messageId: message.id, detail: envelope.detail };
      }
      return { ok: true, frame: { type: 'send', message: envelope.value } };
    }
    case 'ping': {
      const extraKey = findUnexpectedKey(frame, ['type']);
      if (extraKey !== undefined) {
        return badFrame(`ping has an unexpected field: ${echoKey(extraKey)}`);
      }
      return { ok: true, frame: { type: 'ping' } };
    }
    default:
      return badFrame('unknown frame type');
  }
}

/**
 * Parses one text frame from the server, leniently: unknown frame types are
 * reported as `unknown` for the app to ignore, and extra fields are dropped.
 * @param text The frame as received.
 */
export function parseServerFrame(text: string): ServerFrameResult {
  const parsed = parseJson(text);
  if (!parsed.ok || !isRecord(parsed.value) || typeof parsed.value.type !== 'string') {
    return { kind: 'invalid', detail: 'a frame must be a JSON object with a type' };
  }
  const frame = parsed.value;
  switch (frame.type) {
    case 'welcome': {
      const limits = validateLimits(frame.limits);
      if (
        typeof frame.protocol !== 'string' ||
        !isUuid(frame.databaseId) ||
        !isCount(frame.latestSeq) ||
        !isTimestamp(frame.serverTime) ||
        !limits.ok
      ) {
        return { kind: 'invalid', detail: 'welcome has missing or invalid fields' };
      }
      return {
        kind: 'frame',
        frame: {
          type: 'welcome',
          protocol: frame.protocol,
          databaseId: frame.databaseId,
          latestSeq: frame.latestSeq,
          serverTime: frame.serverTime,
          limits: limits.value,
        },
      };
    }
    case 'ack': {
      if (
        !isUuid(frame.id) ||
        !isPositiveInteger(frame.seq) ||
        !isTimestamp(frame.serverReceivedAt) ||
        typeof frame.duplicate !== 'boolean'
      ) {
        return { kind: 'invalid', detail: 'ack has missing or invalid fields' };
      }
      return {
        kind: 'frame',
        frame: {
          type: 'ack',
          id: frame.id,
          seq: frame.seq,
          serverReceivedAt: frame.serverReceivedAt,
          duplicate: frame.duplicate,
        },
      };
    }
    case 'reject': {
      if (!isUuid(frame.id) || !isRejectCode(frame.code) || typeof frame.detail !== 'string') {
        return { kind: 'invalid', detail: 'reject has missing or invalid fields' };
      }
      return { kind: 'frame', frame: { type: 'reject', id: frame.id, code: frame.code, detail: frame.detail } };
    }
    case 'message': {
      const message = validateStoredMessage(frame.message);
      if (!message.ok) {
        return { kind: 'invalid', detail: `message frame: ${message.detail}` };
      }
      return { kind: 'frame', frame: { type: 'message', message: message.value } };
    }
    case 'pong':
      return { kind: 'frame', frame: { type: 'pong' } };
    case 'error': {
      if (!isConnectionErrorCode(frame.code) || typeof frame.detail !== 'string') {
        return { kind: 'invalid', detail: 'error has missing or invalid fields' };
      }
      return { kind: 'frame', frame: { type: 'error', code: frame.code, detail: frame.detail } };
    }
    default:
      return { kind: 'unknown' };
  }
}

function checkEnvelopeFields(record: Record<string, unknown>): Validation<MessageEnvelope> {
  const { id, roomId, senderId, clientCreatedAt, attachmentId, payload } = record;
  if (!isUuid(id)) {
    return fail('id must be a lowercase UUID v4');
  }
  if (!isToken(roomId)) {
    return fail(`roomId must be a string of 1 to ${MAX_TOKEN_LENGTH} characters`);
  }
  if (!isToken(senderId)) {
    return fail(`senderId must be a string of 1 to ${MAX_TOKEN_LENGTH} characters`);
  }
  if (!isTimestamp(clientCreatedAt)) {
    return fail('clientCreatedAt must be an ISO 8601 UTC time with milliseconds');
  }
  if (attachmentId !== null && !isUuid(attachmentId)) {
    return fail('attachmentId must be null or a lowercase UUID v4');
  }
  if (!isRecord(payload) || !isToken(payload.format)) {
    return fail('payload must be an object with a format');
  }
  const opaquePayload: OpaquePayload = { ...payload, format: payload.format };
  return ok({ id, roomId, senderId, clientCreatedAt, attachmentId, payload: opaquePayload });
}

function validatePlainAttachment(value: unknown): Validation<PlainAttachment> {
  if (!isRecord(value)) {
    return fail('payload attachment must be null or an object');
  }
  const extraKey = findUnexpectedKey(value, PLAIN_ATTACHMENT_KEYS);
  if (extraKey !== undefined) {
    return fail(`payload attachment has an unexpected field: ${echoKey(extraKey)}`);
  }
  const { fileName, mimeType, byteSize, sha256 } = value;
  if (typeof fileName !== 'string' || fileName.length === 0 || fileName.length > MAX_FILE_NAME_LENGTH) {
    return fail(`attachment fileName must be a string of 1 to ${MAX_FILE_NAME_LENGTH} characters`);
  }
  if (typeof mimeType !== 'string' || !MIME_TYPE_PATTERN.test(mimeType)) {
    return fail('attachment mimeType must look like type/subtype');
  }
  if (!isCount(byteSize)) {
    return fail('attachment byteSize must be a non-negative integer');
  }
  if (!isSha256Hex(sha256)) {
    return fail('attachment sha256 must be 64 lowercase hex digits');
  }
  return ok({ fileName, mimeType, byteSize, sha256 });
}

function validateLimits(value: unknown): Validation<ChatLimits> {
  if (!isRecord(value)) {
    return fail('limits must be an object');
  }
  const { maxTextLength, maxAttachmentBytes, maxFrameBytes } = value;
  if (!isPositiveInteger(maxTextLength) || !isPositiveInteger(maxAttachmentBytes) || !isPositiveInteger(maxFrameBytes)) {
    return fail('limits must be positive integers');
  }
  return ok({ maxTextLength, maxAttachmentBytes, maxFrameBytes });
}

function parseJson(text: string): Validation<unknown> {
  try {
    return ok(JSON.parse(text));
  } catch {
    return fail('not JSON');
  }
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function findUnexpectedKey(record: Record<string, unknown>, allowedKeys: readonly string[]): string | undefined {
  return Object.keys(record).find((key) => !allowedKeys.includes(key));
}

function echoKey(key: string): string {
  return key.length > MAX_ECHOED_KEY_LENGTH ? `${key.slice(0, MAX_ECHOED_KEY_LENGTH)}…` : key;
}

function isToken(value: unknown): value is string {
  return typeof value === 'string' && value.length > 0 && value.length <= MAX_TOKEN_LENGTH;
}

function isCount(value: unknown): value is number {
  return typeof value === 'number' && Number.isSafeInteger(value) && value >= 0;
}

function isPositiveInteger(value: unknown): value is number {
  return isCount(value) && value > 0;
}

function isRejectCode(value: unknown): value is RejectCode {
  return REJECT_CODES.some((code) => code === value);
}

function isConnectionErrorCode(value: unknown): value is ConnectionErrorCode {
  return CONNECTION_ERROR_CODES.some((code) => code === value);
}

function ok<T>(value: T): Validation<T> {
  return { ok: true, value };
}

function fail<T>(detail: string): Validation<T> {
  return { ok: false, detail };
}

function badFrame(detail: string): ClientFrameResult {
  return { ok: false, error: 'bad_frame', detail };
}
