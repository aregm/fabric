/**
 * Deletes the chat database files (docs/paracels-chat.md, D6). Stop the dev
 * server and the socket server first; a running server keeps writing to the
 * file it already has open.
 */

import { existsSync, rmSync } from 'node:fs';

import { resolveDatabasePath } from '../src/chat/server/config.ts';

/**
 * Deletes the database and its WAL companions.
 * @param filePath The database file.
 * @returns The paths that were deleted.
 */
export function removeDatabaseFiles(filePath: string): string[] {
  const removed: string[] = [];
  for (const suffix of ['', '-wal', '-shm', '-journal']) {
    const candidate = `${filePath}${suffix}`;
    if (!existsSync(candidate)) {
      continue;
    }
    rmSync(candidate);
    removed.push(candidate);
  }
  return removed;
}

/**
 * Runs the reset and reports which files were deleted.
 * @param filePath The database file.
 * @param writeLine Receives each line of output.
 */
export function runChatReset(filePath: string, writeLine: (line: string) => void): void {
  writeLine('Stop the dev server and the socket server before resetting.');
  const removed = removeDatabaseFiles(filePath);
  if (removed.length === 0) {
    writeLine(`No chat database at ${filePath}.`);
    return;
  }
  for (const removedPath of removed) {
    writeLine(`Removed ${removedPath}`);
  }
}

if (import.meta.main) {
  const filePath = resolveDatabasePath(process.env, process.cwd());
  runChatReset(filePath, (line) => {
    process.stdout.write(`${line}\n`);
  });
}
