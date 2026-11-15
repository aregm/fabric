/**
 * Attachment blobs (docs/paracels-chat.md, sections 6.2 and 10). Only the API
 * routes write them (D3); the socket server reads their size and hash for the
 * policy check. The blob table holds bytes, size, hash and uploader only (D9).
 */

import { createHash } from 'node:crypto';

import { readBlob, readInteger, readText, type ChatDatabase } from './database.ts';

/** What the server knows about a stored blob, apart from its bytes. */
export type AttachmentSummary = {
  id: string;
  byteSize: number;
  /** Lowercase hex SHA-256, computed by the server. */
  sha256: string;
};

export type AttachmentReader = {
  /** The stored blob's size and hash, or `null` if no blob has this ID. */
  findAttachment(attachmentId: string): AttachmentSummary | null;
  /** The stored bytes, or `null` if no blob has this ID. */
  readAttachmentBytes(attachmentId: string): Uint8Array<ArrayBuffer> | null;
};

export type AttachmentUpload = {
  id: string;
  /** The demo user who uploaded it, for attribution. */
  uploaderId: string;
  bytes: Uint8Array;
  /** ISO 8601 in UTC with milliseconds. */
  createdAt: string;
};

/** What storing a blob did. */
export type AttachmentOutcome =
  | { kind: 'stored'; attachment: AttachmentSummary }
  /** The same bytes were already stored under this ID. */
  | { kind: 'already_stored'; attachment: AttachmentSummary }
  /** Different bytes are stored under this ID. */
  | { kind: 'conflict' };

export type AttachmentWriter = {
  /** Stores a blob once per ID, computing its SHA-256. */
  storeAttachment(upload: AttachmentUpload): AttachmentOutcome;
};

/**
 * Creates the read side of the blob table.
 * @param database An open chat database.
 */
export function createAttachmentReader(database: ChatDatabase): AttachmentReader {
  const { connection } = database;
  const findSummary = connection.prepare('SELECT id, byte_size, sha256 FROM attachments WHERE id = ?');
  const findBytes = connection.prepare('SELECT bytes FROM attachments WHERE id = ?');

  return {
    findAttachment(attachmentId) {
      const row = findSummary.get(attachmentId);
      return row === undefined ? null : rowToSummary(row);
    },
    readAttachmentBytes(attachmentId) {
      const row = findBytes.get(attachmentId);
      return row === undefined ? null : readBlob(row, 'bytes');
    },
  };
}

/**
 * Creates the write side of the blob table. Only the API routes may call this (D3).
 * @param database An open chat database.
 */
export function createAttachmentWriter(database: ChatDatabase): AttachmentWriter {
  const { connection } = database;
  const insert = connection.prepare(
    `INSERT INTO attachments (id, uploader_id, byte_size, sha256, bytes, created_at)
     VALUES (?, ?, ?, ?, ?, ?)
     ON CONFLICT (id) DO NOTHING`,
  );
  const findSummary = connection.prepare('SELECT id, byte_size, sha256 FROM attachments WHERE id = ?');

  return {
    storeAttachment({ id, uploaderId, bytes, createdAt }) {
      const sha256 = createHash('sha256').update(bytes).digest('hex');
      const { changes } = insert.run(id, uploaderId, bytes.byteLength, sha256, bytes, createdAt);
      const attachment: AttachmentSummary = { id, byteSize: bytes.byteLength, sha256 };
      if (Number(changes) === 1) {
        return { kind: 'stored', attachment };
      }
      const existingRow = findSummary.get(id);
      if (existingRow === undefined) {
        throw new Error(`Attachment ${id} was neither inserted nor found.`);
      }
      const existing = rowToSummary(existingRow);
      return existing.sha256 === sha256 && existing.byteSize === bytes.byteLength
        ? { kind: 'already_stored', attachment: existing }
        : { kind: 'conflict' };
    },
  };
}

function rowToSummary(row: Record<string, unknown>): AttachmentSummary {
  return { id: readText(row, 'id'), byteSize: readInteger(row, 'byte_size'), sha256: readText(row, 'sha256') };
}
