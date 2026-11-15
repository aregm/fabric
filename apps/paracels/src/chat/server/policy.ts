/**
 * The policy module (docs/paracels-chat.md, sections 5.8 and 11). On the server,
 * it alone decides whether a message or upload is acceptable, and it alone reads
 * payload fields. Fabric's policy engine replaces it later without touching
 * transport or storage.
 */

import { CHAT_LIMITS } from '../shared/limits.ts';
import {
  MAIN_ROOM_ID,
  PLAIN_PAYLOAD_FORMAT,
  type MessageEnvelope,
  type OpaquePayload,
  type RejectCode,
} from '../shared/protocol.ts';
import { isDemoUserId, type DemoUserId } from '../shared/users.ts';
import { validatePlainPayload } from '../shared/validate.ts';
import type { AttachmentSummary } from './attachment-store.ts';

/** What the policy module allows the socket server to log about an accepted message. */
export type MessageSummary = {
  /** The text length, or `null` once payloads are encrypted and the length is unknown. */
  textLength: number | null;
  hasAttachment: boolean;
};

export type MessageDecision =
  | { isAccepted: true; summary: MessageSummary }
  | { isAccepted: false; code: RejectCode; detail: string };

export type UploadDecision =
  /** Read at most `maxBytes` of the body, and refuse it as too large beyond that. */
  | { isAccepted: true; uploaderId: DemoUserId; maxBytes: number }
  | { isAccepted: false; code: 'unknown_uploader' | 'attachment_too_large'; detail: string };

/**
 * Decides whether the socket server may log a message.
 * @param envelope A structurally valid envelope (see `validateEnvelope`).
 * @param findAttachment Looks up a stored blob's size and hash.
 */
export function reviewMessage(
  envelope: MessageEnvelope,
  findAttachment: (attachmentId: string) => AttachmentSummary | null,
): MessageDecision {
  if (envelope.roomId !== MAIN_ROOM_ID) {
    return reject('invalid_message', `the only room is ${MAIN_ROOM_ID}`);
  }
  if (!isDemoUserId(envelope.senderId)) {
    return reject('unknown_sender', 'the sender is not a demo user');
  }
  if (envelope.payload.format !== PLAIN_PAYLOAD_FORMAT) {
    return reject('invalid_message', `the payload format must be ${PLAIN_PAYLOAD_FORMAT}`);
  }
  const payload = validatePlainPayload(envelope.payload);
  if (!payload.ok) {
    return reject('invalid_message', payload.detail);
  }
  const { text, attachment } = payload.value;
  if (text.length > CHAT_LIMITS.maxTextLength) {
    return reject('invalid_message', `the text is longer than ${CHAT_LIMITS.maxTextLength} characters`);
  }
  if ((envelope.attachmentId === null) !== (attachment === null)) {
    return reject('invalid_message', 'attachmentId and payload.attachment must be both set or both null');
  }
  if (envelope.attachmentId === null || attachment === null) {
    return text.trim().length === 0
      ? reject('invalid_message', 'a message needs text or an attachment')
      : accept({ textLength: text.length, hasAttachment: false });
  }
  const storedAttachment = findAttachment(envelope.attachmentId);
  if (storedAttachment === null) {
    return reject('attachment_missing', 'no attachment is stored under attachmentId; upload it first');
  }
  if (storedAttachment.byteSize !== attachment.byteSize || storedAttachment.sha256 !== attachment.sha256) {
    return reject('attachment_mismatch', 'the attachment size or SHA-256 differs from the stored upload');
  }
  return accept({ textLength: text.length, hasAttachment: true });
}

/**
 * Decides whether the upload route may read and store an attachment.
 * @param uploaderId The `X-Chat-Uploader` header, or `null` when missing.
 * @param declaredByteSize The `Content-Length`, or `null` when the request has none.
 */
export function reviewUpload(uploaderId: string | null, declaredByteSize: number | null): UploadDecision {
  if (!isDemoUserId(uploaderId)) {
    return { isAccepted: false, code: 'unknown_uploader', detail: 'X-Chat-Uploader must name a demo user.' };
  }
  if (declaredByteSize !== null && declaredByteSize > CHAT_LIMITS.maxAttachmentBytes) {
    return { isAccepted: false, code: 'attachment_too_large', detail: tooLargeDetail() };
  }
  return { isAccepted: true, uploaderId, maxBytes: CHAT_LIMITS.maxAttachmentBytes };
}

/** The text and attachment name of a `plain/1` payload, for the operator log. */
export type PlainMessageContent = {
  text: string;
  attachmentFileName: string | null;
};

/**
 * Reads the text and attachment name of a stored payload.
 * Returns null for anything other than a valid `plain/1` payload, so the log
 * script does not read payload fields itself.
 * @param payload A payload from the message log.
 */
export function readPlainMessageContent(payload: OpaquePayload): PlainMessageContent | null {
  const plain = validatePlainPayload(payload);
  if (!plain.ok) {
    return null;
  }
  return { text: plain.value.text, attachmentFileName: plain.value.attachment?.fileName ?? null };
}

/** The explanation sent with every refusal of an oversized upload. */
export function tooLargeDetail(): string {
  return `Attachments may be at most ${CHAT_LIMITS.maxAttachmentBytes} bytes.`;
}

function accept(summary: MessageSummary): MessageDecision {
  return { isAccepted: true, summary };
}

function reject(code: RejectCode, detail: string): MessageDecision {
  return { isAccepted: false, code, detail };
}
