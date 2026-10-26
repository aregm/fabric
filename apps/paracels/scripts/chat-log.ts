/**
 * Prints the stored messages (docs/paracels-chat.md, section 5.7): sequence
 * number, server time, sender, attachment name and text. Payload fields are
 * read through the policy module.
 */

import { existsSync } from 'node:fs';

import { HISTORY_MAX_PAGE_SIZE } from '../src/chat/shared/limits.ts';
import { MAIN_ROOM_ID, type StoredMessage } from '../src/chat/shared/protocol.ts';
import { findDemoUser } from '../src/chat/shared/users.ts';
import { resolveDatabasePath } from '../src/chat/server/config.ts';
import { openChatDatabase, type ChatDatabase } from '../src/chat/server/database.ts';
import { createMessageReader } from '../src/chat/server/message-store.ts';
import { readPlainMessageContent } from '../src/chat/server/policy.ts';

/**
 * Renders the message log as text.
 * @param databasePath The database file.
 * @returns The log, or a short note when the database is missing or empty.
 */
export function renderChatLog(databasePath: string): string {
  if (!existsSync(databasePath)) {
    return `No chat database at ${databasePath}.`;
  }
  const database = openChatDatabase(databasePath, { readOnly: true });
  try {
    return renderMessages(database);
  } finally {
    database.connection.close();
  }
}

function renderMessages(database: ChatDatabase): string {
  const reader = createMessageReader(database);
  const blocks: string[] = [];
  let after = 0;
  for (;;) {
    const page = reader.readHistory({ roomId: MAIN_ROOM_ID, limit: HISTORY_MAX_PAGE_SIZE, cursor: { after } });
    if (page.messages.length === 0) {
      break;
    }
    for (const message of page.messages) {
      blocks.push(formatMessage(message));
      after = message.seq;
    }
    if (!page.hasMore) {
      break;
    }
  }
  return blocks.length === 0 ? 'No messages stored.' : blocks.join('\n\n');
}

function formatMessage(message: StoredMessage): string {
  const sender = findDemoUser(message.senderId)?.displayName ?? message.senderId;
  const content = readPlainMessageContent(message.payload);
  const heading = `#${message.seq}  ${message.serverReceivedAt}  ${sender}`;
  if (content === null) {
    return `${heading}\n(unsupported payload)`;
  }
  const attachment = content.attachmentFileName === null ? '' : `\nAttachment: ${content.attachmentFileName}`;
  return `${heading}${attachment}\n${content.text}`;
}

if (import.meta.main) {
  try {
    const text = renderChatLog(resolveDatabasePath(process.env, process.cwd()));
    process.stdout.write(`${text}\n`);
  } catch (error) {
    process.stderr.write(`${error instanceof Error ? error.message : 'Could not read the chat database.'}\n`);
    process.exitCode = 1;
  }
}
