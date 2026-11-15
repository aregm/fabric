/**
 * Frames, envelopes and HTTP shapes of the `paracels-chat/1` protocol
 * (docs/paracels-chat.md, sections 6 and 7). A change to any of them needs a
 * new protocol or payload version.
 */

import type { ChatLimits } from './limits.ts';

/** Sent in `hello` and `welcome`, and returned by `GET /api/health`. */
export const PROTOCOL_VERSION = 'paracels-chat/1';

/** The format tag of unencrypted payloads (D9). */
export const PLAIN_PAYLOAD_FORMAT = 'plain/1';

/** The only room (D10). A Fabric space ID takes its place later. */
export const MAIN_ROOM_ID = 'main';

/** The request header that names the demo user uploading an attachment. */
export const UPLOADER_HEADER = 'X-Chat-Uploader';

/** The socket server's port unless `CHAT_SOCKET_PORT` says otherwise (D1). */
export const DEFAULT_SOCKET_PORT = 8787;

/** The path the socket server accepts WebSocket connections on. */
export const SOCKET_PATH = '/chat';

/** Attachment metadata inside a `plain/1` payload. */
export type PlainAttachment = {
  fileName: string;
  mimeType: string;
  /** Size in bytes, as returned by the upload. */
  byteSize: number;
  /** Lowercase hex SHA-256 of the bytes, as returned by the upload. */
  sha256: string;
};

/** The content of an unencrypted message. */
export type PlainPayload = {
  format: typeof PLAIN_PAYLOAD_FORMAT;
  text: string;
  attachment: PlainAttachment | null;
};

/**
 * A payload as routing and storage see it: a format tag, with every other field
 * opaque. On the server, only the policy module reads further.
 */
export type OpaquePayload = {
  readonly format: string;
  readonly [field: string]: unknown;
};

/** A message as the device creates it (section 6.1). */
export type MessageEnvelope = {
  /** Lowercase UUID v4, kept through every retry. */
  id: string;
  roomId: string;
  senderId: string;
  /** ISO 8601 in UTC with milliseconds. */
  clientCreatedAt: string;
  /** The uploaded blob this message refers to, or `null`. */
  attachmentId: string | null;
  payload: OpaquePayload;
};

/** A logged message: the envelope plus the order and time the socket server assigned. */
export type StoredMessage = MessageEnvelope & {
  /** Positive, increasing and never reused; orders the room. */
  seq: number;
  /** ISO 8601 in UTC with milliseconds. */
  serverReceivedAt: string;
};

export type HelloFrame = { type: 'hello'; protocol: string; roomKey: string };
export type SendFrame = { type: 'send'; message: MessageEnvelope };
export type PingFrame = { type: 'ping' };

/** A frame from the app to the socket server. */
export type ClientFrame = HelloFrame | SendFrame | PingFrame;

export const REJECT_CODES = [
  'invalid_message',
  'unknown_sender',
  'attachment_missing',
  'attachment_mismatch',
  'id_conflict',
] as const;

/** Why the server refused one message. The connection stays open. */
export type RejectCode = (typeof REJECT_CODES)[number];

export const CONNECTION_ERROR_CODES = [
  'bad_frame',
  'unsupported_protocol',
  'unauthorized',
  'hello_timeout',
  'not_ready',
  'internal_error',
] as const;

/** Why the server is closing a connection. */
export type ConnectionErrorCode = (typeof CONNECTION_ERROR_CODES)[number];

export type WelcomeFrame = {
  type: 'welcome';
  protocol: string;
  databaseId: string;
  latestSeq: number;
  serverTime: string;
  limits: ChatLimits;
};
export type AckFrame = {
  type: 'ack';
  id: string;
  seq: number;
  serverReceivedAt: string;
  /** True when the server had already stored this message ID. */
  duplicate: boolean;
};
export type RejectFrame = { type: 'reject'; id: string; code: RejectCode; detail: string };
export type MessageFrame = { type: 'message'; message: StoredMessage };
export type PongFrame = { type: 'pong' };
export type ErrorFrame = { type: 'error'; code: ConnectionErrorCode; detail: string };

/** A frame from the socket server to the app. */
export type ServerFrame = WelcomeFrame | AckFrame | RejectFrame | MessageFrame | PongFrame | ErrorFrame;

/** WebSocket close codes (section 7.3). */
export const CLOSE_CODES = Object.freeze({
  normal: 1000,
  serverShuttingDown: 1001,
  frameTooLarge: 1009,
  internalError: 1011,
  badFrame: 4400,
  unauthorized: 4401,
  helloTimeout: 4408,
});

export const HTTP_ERROR_CODES = [
  'unauthorized',
  'not_configured',
  'invalid_query',
  'invalid_attachment_id',
  'unknown_uploader',
  'invalid_content_length',
  'incomplete_upload',
  'attachment_conflict',
  'attachment_too_large',
  'not_found',
  'internal_error',
] as const;

export type HttpErrorCode = (typeof HTTP_ERROR_CODES)[number];

/** The body of every HTTP error response. */
export type ApiErrorBody = { error: { code: HttpErrorCode; message: string } };

/** `GET /api/health`. */
export type HealthResponse = { ok: true; protocol: string };

/** `GET /api/messages`: one page, in ascending `seq`. */
export type HistoryPage = {
  databaseId: string;
  messages: StoredMessage[];
  /** The highest `seq` in the room, or 0 when it is empty. */
  latestSeq: number;
  /** True when more messages lie beyond this page in the direction it was read. */
  hasMore: boolean;
};

/** `PUT /api/attachments/{id}`. */
export type UploadResult = { id: string; byteSize: number; sha256: string };
