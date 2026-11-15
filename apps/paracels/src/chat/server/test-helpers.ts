/**
 * Helpers for the server tests. Nothing outside test files imports this module.
 */

import { randomBytes, randomUUID } from 'node:crypto';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import type { TestContext } from 'node:test';

import type { MessageEnvelope, OpaquePayload } from '../shared/protocol.ts';
import { openChatDatabase, type ChatDatabase } from './database.ts';

/**
 * Creates a directory that is deleted when the test ends.
 * @param context The running test.
 */
export function createTemporaryDirectory(context: TestContext): string {
  const directory = mkdtempSync(join(tmpdir(), 'paracels-chat-'));
  context.after(() => rmSync(directory, { recursive: true, force: true }));
  return directory;
}

/**
 * Opens a fresh database file that is closed and deleted when the test ends.
 * @param context The running test.
 */
export function openTemporaryDatabase(context: TestContext): ChatDatabase {
  const database = openChatDatabase(join(createTemporaryDirectory(context), 'chat.sqlite'));
  context.after(() => {
    if (database.connection.isOpen) {
      database.connection.close();
    }
  });
  return database;
}

/** A random key in the form `chat:setup` creates, so tests can search their output for it. */
export function createTestRoomKey(): string {
  return randomBytes(32).toString('base64url');
}

/**
 * Builds a valid text message from Alice, with a fresh ID.
 * @param overrides Fields to replace.
 */
export function makeEnvelope(overrides: Partial<MessageEnvelope> = {}): MessageEnvelope {
  return {
    id: randomUUID(),
    roomId: 'main',
    senderId: 'alice',
    clientCreatedAt: new Date().toISOString(),
    attachmentId: null,
    payload: makePlainPayload('Hello'),
    ...overrides,
  };
}

/**
 * Builds a `plain/1` payload.
 * @param text The message text.
 * @param attachment Attachment metadata, or `null`.
 */
export function makePlainPayload(
  text: string,
  attachment: { fileName: string; mimeType: string; byteSize: number; sha256: string } | null = null,
): OpaquePayload {
  return { format: 'plain/1', text, attachment };
}
