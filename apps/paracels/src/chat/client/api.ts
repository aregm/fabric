/**
 * History over HTTP, plus the upload response the file uploader parses
 * (docs/paracels-chat.md, sections 7.1 and 10). The bytes themselves travel
 * through `expo-file-system`, which the tests do not load.
 */

import { HISTORY_MAX_PAGE_SIZE, HISTORY_DEFAULT_PAGE_SIZE } from '../shared/limits.ts';
import { MAIN_ROOM_ID, type HistoryPage, type HttpErrorCode } from '../shared/protocol.ts';
import { validateStoredMessage } from '../shared/validate.ts';

export type HistoryCursor = { before: number } | { after: number } | null;

export type HistoryQuery = {
  limit?: number;
  cursor?: HistoryCursor;
};

export type ChatApiError = {
  kind: 'http';
  status: number;
  code: HttpErrorCode | 'unknown';
  message: string;
};

export type NetworkError = { kind: 'network'; message: string };

export type ApiFailure = ChatApiError | NetworkError;

export type ApiResult<T> = { ok: true; value: T } | { ok: false; error: ApiFailure };

export type ChatApi = {
  readHistory(query?: HistoryQuery): Promise<ApiResult<HistoryPage>>;
};

/** What `PUT /api/attachments/{id}` means for the outbox. */
export type UploadOutcome =
  | { ok: true; byteSize: number; sha256: string }
  | { ok: false; retryable: boolean; code: string };

export type ChatApiOptions = {
  roomKey: string;
  /** Empty means relative URLs, which Expo resolves against the bundle host. */
  apiOrigin?: string;
  fetch?: typeof fetch;
};

/**
 * Creates the HTTP client.
 * @param options See {@link ChatApiOptions}.
 */
export function createChatApi(options: ChatApiOptions): ChatApi {
  const runFetch = options.fetch ?? fetch;
  const origin = options.apiOrigin ?? '';

  async function readHistory(query: HistoryQuery = {}): Promise<ApiResult<HistoryPage>> {
    const parameters = new URLSearchParams({ roomId: MAIN_ROOM_ID });
    const limit = query.limit ?? HISTORY_DEFAULT_PAGE_SIZE;
    parameters.set('limit', String(Math.min(HISTORY_MAX_PAGE_SIZE, Math.max(1, limit))));
    if (query.cursor !== undefined && query.cursor !== null) {
      if ('before' in query.cursor) {
        parameters.set('before', String(query.cursor.before));
      } else {
        parameters.set('after', String(query.cursor.after));
      }
    }
    const response = await request(runFetch, `${origin}/api/messages?${parameters}`, options.roomKey);
    if (!response.ok) {
      return response;
    }
    return parseHistoryPage(response.value);
  }

  return { readHistory };
}

async function request(
  runFetch: typeof fetch,
  url: string,
  roomKey: string,
  init: RequestInit = {},
): Promise<ApiResult<unknown>> {
  let response: Response;
  try {
    response = await runFetch(url, {
      ...init,
      headers: { Authorization: `Bearer ${roomKey}`, ...init.headers },
    });
  } catch (error) {
    return {
      ok: false,
      error: { kind: 'network', message: error instanceof Error ? error.message : 'The request did not complete.' },
    };
  }
  let body: unknown = null;
  const text = await response.text();
  if (text.length > 0) {
    try {
      body = JSON.parse(text);
    } catch {
      body = null;
    }
  }
  if (!response.ok) {
    return { ok: false, error: readApiError(response.status, body) };
  }
  return { ok: true, value: body };
}

function parseHistoryPage(body: unknown): ApiResult<HistoryPage> {
  if (!isRecord(body) || typeof body.databaseId !== 'string' || typeof body.latestSeq !== 'number' || typeof body.hasMore !== 'boolean' || !Array.isArray(body.messages)) {
    return { ok: false, error: { kind: 'http', status: 200, code: 'unknown', message: 'The history page was malformed.' } };
  }
  const messages = [];
  for (const raw of body.messages) {
    const message = validateStoredMessage(raw);
    if (!message.ok) {
      return { ok: false, error: { kind: 'http', status: 200, code: 'unknown', message: 'The history page had a malformed message.' } };
    }
    messages.push(message.value);
  }
  return {
    ok: true,
    value: { databaseId: body.databaseId, messages, latestSeq: body.latestSeq, hasMore: body.hasMore },
  };
}

function readApiError(status: number, body: unknown): ChatApiError {
  if (isRecord(body) && isRecord(body.error) && typeof body.error.code === 'string' && typeof body.error.message === 'string') {
    return { kind: 'http', status, code: body.error.code as HttpErrorCode, message: body.error.message };
  }
  return { kind: 'http', status, code: 'unknown', message: `The server answered ${status}.` };
}

/**
 * The URL of one attachment on the API origin.
 * @param apiOrigin The bundle origin, such as `http://192.168.1.185:8081`.
 * @param attachmentId The attachment's UUID.
 */
export function attachmentUrl(apiOrigin: string, attachmentId: string): string {
  return `${apiOrigin}/api/attachments/${attachmentId}`;
}

const SHA256_PATTERN = /^[0-9a-f]{64}$/;

/**
 * Reads the upload route's answer. A network failure never reaches this function.
 * @param status The HTTP status.
 * @param body The response body, which is JSON on success.
 */
export function readUploadResponse(status: number, body: string): UploadOutcome {
  if (status === 200 || status === 201) {
    let parsed: unknown;
    try {
      parsed = JSON.parse(body);
    } catch {
      return { ok: false, retryable: false, code: 'upload_failed' };
    }
    if (!isRecord(parsed) || typeof parsed.sha256 !== 'string' || !SHA256_PATTERN.test(parsed.sha256) || !isByteSize(parsed.byteSize)) {
      return { ok: false, retryable: false, code: 'upload_failed' };
    }
    return { ok: true, byteSize: parsed.byteSize, sha256: parsed.sha256 };
  }
  const code = readApiError(status, parseJson(body)).code;
  return { ok: false, retryable: status === 408 || status === 429 || status >= 500, code };
}

function parseJson(body: string): unknown {
  try {
    return JSON.parse(body);
  } catch {
    return null;
  }
}

function isByteSize(value: unknown): value is number {
  return typeof value === 'number' && Number.isSafeInteger(value) && value >= 0;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}
