/**
 * Remembers the selected demo user on this device (D16).
 */

import Storage from 'expo-sqlite/kv-store';

import { isDemoUserId, type DemoUserId } from '../shared/users.ts';

const USER_KEY = 'paracels-chat.userId';

/**
 * Reads the last selected demo user, or Alice when none is stored.
 */
export function readStoredUserId(): DemoUserId {
  const stored = Storage.getItemSync(USER_KEY);
  return isDemoUserId(stored) ? stored : 'alice';
}

/**
 * Remembers the selected demo user.
 * @param userId Alice or Bob.
 */
export function writeStoredUserId(userId: DemoUserId): void {
  Storage.setItemSync(USER_KEY, userId);
}
