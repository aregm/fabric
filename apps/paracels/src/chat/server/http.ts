/**
 * The handlers behind the API routes (docs/paracels-chat.md, section 7.1). Each
 * takes a Web `Request` and returns a `Response`, so tests call them directly,
 * without a running server. They write only the attachments table (D3).
 */

import { HISTORY_DEFAULT_PAGE_SIZE, HISTORY_MAX_PAGE_SIZE } from '../shared/limits.ts';
import {
  MAIN_ROOM_ID,
  PROTOCOL_VERSION,
  UPLOADER_HEADER,
  type ApiErrorBody,
  type HealthResponse,
  type HistoryPage,
  type HttpErrorCode,
  type UploadResult,
} from '../shared/protocol.ts';
import { isUuid, type Validation } from '../shared/validate.ts';
import type { AttachmentReader, AttachmentWriter } from './attachment-store.ts';
import type { ChatLogger } from './log.ts';
import type { HistoryQuery, MessageReader } from './message-store.ts';
import { reviewUpload, tooLargeDetail } from './policy.ts';
import { readBearerToken, type RoomKeyCheck } from './room-key.ts';

/** What the handlers need from the process that runs them. */
export type HttpContext = {
  /** Checks the presented room key; `null` when none is configured, so every request gets 503. */
  isRoomKey: RoomKeyCheck | null;
  databaseId: string;
  messages: MessageReader;
  attachments: AttachmentReader & AttachmentWriter;
  log: ChatLogger;
  now: () => Date;
};

type BodyRead = { kind: 'complete'; bytes: Uint8Array } | { kind: 'too_large' } | { kind: 'incomplete' };

const DOWNLOAD_HEADERS = Object.freeze({
  'Content-Type': 'application/octet-stream',
  'Content-Disposition': 'attachment',
  'X-Content-Type-Options': 'nosniff',
  'Cache-Control': 'private, max-age=31536000, immutable',
});

/**
 * `GET /api/health`: tells the app that the server is reachable and its key is right.
 * @param request The incoming request.
 * @param context See {@link HttpContext}.
 */
export async function handleHealth(request: Request, context: HttpContext): Promise<Response> {
  const refusal = authorize(request, context);
  if (refusal !== null) {
    return refusal;
  }
  const body: HealthResponse = { ok: true, protocol: PROTOCOL_VERSION };
  return jsonResponse(200, body);
}

/**
 * `GET /api/messages`: one page of history, in ascending `seq`.
 * Query: `roomId` (always `main`), `limit` (1–200, default 50), and either `before` or `after`.
 * @param request The incoming request.
 * @param context See {@link HttpContext}.
 */
export async function handleHistory(request: Request, context: HttpContext): Promise<Response> {
  const refusal = authorize(request, context);
  if (refusal !== null) {
    return refusal;
  }
  const query = parseHistoryQuery(new URL(request.url).searchParams);
  if (!query.ok) {
    return errorResponse(400, 'invalid_query', query.detail);
  }
  const page = context.messages.readHistory(query.value);
  const body: HistoryPage = {
    databaseId: context.databaseId,
    messages: page.messages,
    latestSeq: page.latestSeq,
    hasMore: page.hasMore,
  };
  return jsonResponse(200, body);
}

/**
 * `PUT /api/attachments/{id}`: stores one attachment's bytes, sent as the raw body.
 * The size limit is checked against `Content-Length` before reading, and again
 * while reading, so a missing or wrong length cannot get past it.
 * @param request The incoming request; `X-Chat-Uploader` names the demo user.
 * @param attachmentId The `{id}` route segment.
 * @param context See {@link HttpContext}.
 */
export async function handleAttachmentUpload(
  request: Request,
  attachmentId: string,
  context: HttpContext,
): Promise<Response> {
  const refusal = authorize(request, context);
  if (refusal !== null) {
    return refusal;
  }
  if (!isUuid(attachmentId)) {
    return errorResponse(400, 'invalid_attachment_id', 'The attachment ID must be a lowercase UUID v4.');
  }
  const declaredLength = parseContentLength(request.headers.get('content-length'));
  if (!declaredLength.ok) {
    return errorResponse(400, 'invalid_content_length', declaredLength.detail);
  }
  const decision = reviewUpload(request.headers.get(UPLOADER_HEADER), declaredLength.value);
  if (!decision.isAccepted) {
    context.log('attachment_refused', { id: attachmentId, code: decision.code });
    return decision.code === 'attachment_too_large'
      ? errorResponse(413, decision.code, decision.detail)
      : errorResponse(400, decision.code, decision.detail);
  }
  const body = await readBody(request, decision.maxBytes);
  if (body.kind === 'too_large') {
    context.log('attachment_refused', { id: attachmentId, code: 'attachment_too_large' });
    return errorResponse(413, 'attachment_too_large', tooLargeDetail());
  }
  if (body.kind === 'incomplete' || (declaredLength.value !== null && body.bytes.byteLength !== declaredLength.value)) {
    return errorResponse(400, 'incomplete_upload', 'The upload ended before all its bytes arrived.');
  }
  const outcome = context.attachments.storeAttachment({
    id: attachmentId,
    uploaderId: decision.uploaderId,
    bytes: body.bytes,
    createdAt: context.now().toISOString(),
  });
  if (outcome.kind === 'conflict') {
    context.log('attachment_refused', { id: attachmentId, code: 'attachment_conflict' });
    return errorResponse(409, 'attachment_conflict', 'Different bytes are already stored under this attachment ID.');
  }
  const result: UploadResult = {
    id: outcome.attachment.id,
    byteSize: outcome.attachment.byteSize,
    sha256: outcome.attachment.sha256,
  };
  if (outcome.kind === 'stored') {
    context.log('attachment_stored', { id: result.id, byteSize: result.byteSize, uploaderId: decision.uploaderId });
    return jsonResponse(201, result);
  }
  return jsonResponse(200, result);
}

/**
 * `GET /api/attachments/{id}`: the stored bytes, sent so that a browser never renders them.
 * @param request The incoming request.
 * @param attachmentId The `{id}` route segment.
 * @param context See {@link HttpContext}.
 */
export async function handleAttachmentDownload(
  request: Request,
  attachmentId: string,
  context: HttpContext,
): Promise<Response> {
  const refusal = authorize(request, context);
  if (refusal !== null) {
    return refusal;
  }
  const bytes = isUuid(attachmentId) ? context.attachments.readAttachmentBytes(attachmentId) : null;
  if (bytes === null) {
    return errorResponse(404, 'not_found', 'No attachment is stored under this ID.');
  }
  return new Response(bytes, { status: 200, headers: DOWNLOAD_HEADERS });
}

/** The response for an error no handler expected, without its details. */
export function internalErrorResponse(): Response {
  return errorResponse(500, 'internal_error', 'The chat server hit an unexpected error.');
}

function authorize(request: Request, context: HttpContext): Response | null {
  if (context.isRoomKey === null) {
    return errorResponse(
      503,
      'not_configured',
      'The chat room key is not set. Run npm run chat:setup, then restart the dev server.',
    );
  }
  const presentedKey = readBearerToken(request.headers.get('authorization'));
  if (context.isRoomKey(presentedKey)) {
    return null;
  }
  context.log('http_unauthorized', {
    method: request.method,
    path: new URL(request.url).pathname,
    reason: presentedKey === null ? 'missing_key' : 'wrong_key',
  });
  return errorResponse(401, 'unauthorized', 'This request needs the room key.', { 'WWW-Authenticate': 'Bearer' });
}

function parseHistoryQuery(parameters: URLSearchParams): Validation<HistoryQuery> {
  for (const name of ['roomId', 'limit', 'before', 'after']) {
    if (parameters.getAll(name).length > 1) {
      return { ok: false, detail: `${name} may appear only once.` };
    }
  }
  const roomId = parameters.get('roomId');
  if (roomId !== MAIN_ROOM_ID) {
    return { ok: false, detail: `roomId is required and must be ${MAIN_ROOM_ID}.` };
  }
  const limitText = parameters.get('limit');
  const limit = limitText === null ? HISTORY_DEFAULT_PAGE_SIZE : parseCount(limitText);
  if (limit === null || limit < 1 || limit > HISTORY_MAX_PAGE_SIZE) {
    return { ok: false, detail: `limit must be a whole number from 1 to ${HISTORY_MAX_PAGE_SIZE}.` };
  }
  const beforeText = parameters.get('before');
  const afterText = parameters.get('after');
  if (beforeText !== null && afterText !== null) {
    return { ok: false, detail: 'Use before or after, not both.' };
  }
  if (beforeText === null && afterText === null) {
    return { ok: true, value: { roomId, limit, cursor: null } };
  }
  const seq = parseCount(beforeText ?? afterText ?? '');
  if (seq === null) {
    return { ok: false, detail: 'before and after must be sequence numbers (whole numbers, 0 or more).' };
  }
  return { ok: true, value: { roomId, limit, cursor: beforeText !== null ? { before: seq } : { after: seq } } };
}

function parseContentLength(headerValue: string | null): Validation<number | null> {
  if (headerValue === null) {
    return { ok: true, value: null };
  }
  const byteCount = parseCount(headerValue.trim());
  return byteCount === null
    ? { ok: false, detail: 'Content-Length must be a whole number of bytes.' }
    : { ok: true, value: byteCount };
}

/** Parses a non-negative decimal integer, or returns `null`. */
function parseCount(text: string): number | null {
  if (!/^\d{1,15}$/.test(text)) {
    return null;
  }
  return Number(text);
}

async function readBody(request: Request, maxBytes: number): Promise<BodyRead> {
  if (request.body === null) {
    return { kind: 'complete', bytes: new Uint8Array(0) };
  }
  const reader = request.body.getReader();
  const chunks: Uint8Array[] = [];
  let byteCount = 0;
  try {
    for (;;) {
      const { done, value } = await reader.read();
      if (done) {
        break;
      }
      byteCount += value.byteLength;
      if (byteCount > maxBytes) {
        await reader.cancel().catch(() => undefined);
        return { kind: 'too_large' };
      }
      chunks.push(value);
    }
  } catch {
    return { kind: 'incomplete' };
  }
  const bytes = new Uint8Array(byteCount);
  let offset = 0;
  for (const chunk of chunks) {
    bytes.set(chunk, offset);
    offset += chunk.byteLength;
  }
  return { kind: 'complete', bytes };
}

function jsonResponse(status: number, body: unknown, headers: Readonly<Record<string, string>> = {}): Response {
  return Response.json(body, { status, headers: { 'Cache-Control': 'no-store', ...headers } });
}

function errorResponse(
  status: number,
  code: HttpErrorCode,
  message: string,
  headers: Readonly<Record<string, string>> = {},
): Response {
  const body: ApiErrorBody = { error: { code, message } };
  return jsonResponse(status, body, headers);
}
