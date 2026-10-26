/**
 * The message log (docs/paracels-chat.md, sections 6.2 and 8). Only the socket
 * server writes it (D3); the API routes and `chat:log` read it.
 */

import type { MessageEnvelope, StoredMessage } from '../shared/protocol.ts';
import { validateStoredMessage } from '../shared/validate.ts';
import { inTransaction, readInteger, readNullableText, readText, type ChatDatabase } from './database.ts';

/** Where a history page starts: the newest messages, or before or after a sequence number. */
export type HistoryCursor = { before: number } | { after: number } | null;

export type HistoryQuery = {
  roomId: string;
  /** Messages per page, already checked against the history limits. */
  limit: number;
  cursor: HistoryCursor;
};

export type HistoryResult = {
  /** In ascending `seq`. */
  messages: StoredMessage[];
  /** The highest `seq` in the room, or 0 when it is empty. */
  latestSeq: number;
  /** True when more messages lie beyond this page in the direction it was read. */
  hasMore: boolean;
};

export type MessageReader = {
  /**
   * Reads one page from a single snapshot, so `latestSeq` matches the page.
   * Without a cursor it returns the newest messages; `hasMore` then means older ones exist.
   */
  readHistory(query: HistoryQuery): HistoryResult;
  /** The highest `seq` in a room, or 0 when it is empty. */
  readLatestSeq(roomId: string): number;
};

/** What storing a message did. */
export type StoreOutcome =
  /** Newly logged; relay it. */
  | { kind: 'stored'; message: StoredMessage }
  /** The same message was already logged; acknowledge it again, but don't relay it. */
  | { kind: 'duplicate'; message: StoredMessage }
  /** A different message already has this ID. */
  | { kind: 'conflict' };

export type MessageWriter = {
  /**
   * Logs a message once per ID. Returns only after the insert has committed.
   * @param envelope A message that passed validation and the policy module.
   * @param serverReceivedAt The receive time, ISO 8601 in UTC with milliseconds.
   */
  storeMessage(envelope: MessageEnvelope, serverReceivedAt: string): StoreOutcome;
};

const COLUMNS =
  'seq, id, room_id, sender_id, client_created_at, server_received_at, attachment_id, payload_format, payload_json';

/**
 * Creates the read side of the log.
 * @param database An open chat database.
 */
export function createMessageReader(database: ChatDatabase): MessageReader {
  const { connection } = database;
  const newest = connection.prepare(`SELECT ${COLUMNS} FROM messages WHERE room_id = ? ORDER BY seq DESC LIMIT ?`);
  const before = connection.prepare(
    `SELECT ${COLUMNS} FROM messages WHERE room_id = ? AND seq < ? ORDER BY seq DESC LIMIT ?`,
  );
  const after = connection.prepare(`SELECT ${COLUMNS} FROM messages WHERE room_id = ? AND seq > ? ORDER BY seq ASC LIMIT ?`);
  const latest = connection.prepare('SELECT COALESCE(MAX(seq), 0) AS latest_seq FROM messages WHERE room_id = ?');

  function readLatestSeq(roomId: string): number {
    return readInteger(latest.get(roomId) ?? {}, 'latest_seq');
  }

  function readHistory({ roomId, limit, cursor }: HistoryQuery): HistoryResult {
    return inTransaction(connection, 'DEFERRED', () => {
      // One extra row tells whether more lie beyond the page.
      const rowLimit = limit + 1;
      let rows: Record<string, unknown>[];
      let isAscending: boolean;
      if (cursor === null) {
        rows = newest.all(roomId, rowLimit);
        isAscending = false;
      } else if ('before' in cursor) {
        rows = before.all(roomId, cursor.before, rowLimit);
        isAscending = false;
      } else {
        rows = after.all(roomId, cursor.after, rowLimit);
        isAscending = true;
      }
      const pageRows = rows.slice(0, limit);
      if (!isAscending) {
        pageRows.reverse();
      }
      return {
        messages: pageRows.map(rowToMessage),
        latestSeq: readLatestSeq(roomId),
        hasMore: rows.length > limit,
      };
    });
  }

  return { readHistory, readLatestSeq };
}

/**
 * Creates the write side of the log. Only the socket server may call this (D3).
 * @param database An open chat database.
 */
export function createMessageWriter(database: ChatDatabase): MessageWriter {
  const { connection } = database;
  const insert = connection.prepare(
    `INSERT INTO messages
       (id, room_id, sender_id, client_created_at, server_received_at, attachment_id, payload_format, payload_json)
     VALUES (?, ?, ?, ?, ?, ?, ?, ?)
     ON CONFLICT (id) DO NOTHING
     RETURNING ${COLUMNS}`,
  );
  const findById = connection.prepare(`SELECT ${COLUMNS} FROM messages WHERE id = ?`);

  function storeMessage(envelope: MessageEnvelope, serverReceivedAt: string): StoreOutcome {
    const payloadJson = JSON.stringify(envelope.payload);
    const inserted = insert.get(
      envelope.id,
      envelope.roomId,
      envelope.senderId,
      envelope.clientCreatedAt,
      serverReceivedAt,
      envelope.attachmentId,
      envelope.payload.format,
      payloadJson,
    );
    if (inserted !== undefined) {
      return { kind: 'stored', message: rowToMessage(inserted) };
    }
    const existing = findById.get(envelope.id);
    if (existing === undefined) {
      throw new Error(`Message ${envelope.id} was neither inserted nor found.`);
    }
    const isSameMessage =
      readText(existing, 'room_id') === envelope.roomId &&
      readText(existing, 'sender_id') === envelope.senderId &&
      readText(existing, 'client_created_at') === envelope.clientCreatedAt &&
      readNullableText(existing, 'attachment_id') === envelope.attachmentId &&
      readText(existing, 'payload_format') === envelope.payload.format &&
      readText(existing, 'payload_json') === payloadJson;
    return isSameMessage ? { kind: 'duplicate', message: rowToMessage(existing) } : { kind: 'conflict' };
  }

  return { storeMessage };
}

function rowToMessage(row: Record<string, unknown>): StoredMessage {
  const seq = readInteger(row, 'seq');
  let payload: unknown;
  try {
    payload = JSON.parse(readText(row, 'payload_json'));
  } catch {
    throw new Error(`Message ${seq} has a payload that is not JSON.`);
  }
  const message = validateStoredMessage({
    id: readText(row, 'id'),
    roomId: readText(row, 'room_id'),
    senderId: readText(row, 'sender_id'),
    clientCreatedAt: readText(row, 'client_created_at'),
    attachmentId: readNullableText(row, 'attachment_id'),
    payload,
    seq,
    serverReceivedAt: readText(row, 'server_received_at'),
  });
  if (!message.ok) {
    throw new Error(`Message ${seq} in the database is malformed: ${message.detail}.`);
  }
  return message.value;
}
