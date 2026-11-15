/**
 * The shared room key (docs/paracels-chat.md, Q18 and D11). It travels only in
 * the `Authorization` header and the `hello` frame, and is never logged.
 */

import { createHash, timingSafeEqual } from 'node:crypto';

/** Tells whether a presented key is the room key. */
export type RoomKeyCheck = (presentedKey: string | null) => boolean;

const BEARER_PATTERN = /^Bearer +(\S+) *$/i;

/**
 * Creates a check that compares keys in constant time. Both sides are hashed
 * first, so the comparison doesn't depend on the presented key's length either.
 * @param roomKey The configured key.
 */
export function createRoomKeyCheck(roomKey: string): RoomKeyCheck {
  const expectedDigest = sha256(roomKey);
  return (presentedKey) => presentedKey !== null && timingSafeEqual(sha256(presentedKey), expectedDigest);
}

/**
 * Extracts the key from an `Authorization: Bearer <key>` header.
 * @param headerValue The header, or `null` when the request has none.
 * @returns The key, or `null` when the header is missing or uses another scheme.
 */
export function readBearerToken(headerValue: string | null): string | null {
  const match = headerValue === null ? null : BEARER_PATTERN.exec(headerValue);
  return match === null ? null : match[1];
}

function sha256(value: string): Buffer {
  return createHash('sha256').update(value, 'utf8').digest();
}
