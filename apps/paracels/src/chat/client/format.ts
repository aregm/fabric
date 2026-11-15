/**
 * Times and names as the chat screen shows them (D15).
 */

import { findDemoUser } from '../shared/users.ts';

/**
 * Formats a stored time in the device's locale. Same calendar day shows the
 * time only; older days include the date.
 * @param iso An ISO 8601 UTC time.
 * @param now The current instant.
 */
export function formatMessageTime(iso: string, now: Date = new Date()): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) {
    return iso;
  }
  const sameDay =
    date.getFullYear() === now.getFullYear() &&
    date.getMonth() === now.getMonth() &&
    date.getDate() === now.getDate();
  return sameDay
    ? date.toLocaleTimeString(undefined, { hour: 'numeric', minute: '2-digit' })
    : date.toLocaleString(undefined, { month: 'short', day: 'numeric', hour: 'numeric', minute: '2-digit' });
}

/**
 * The name shown for a sender ID.
 * @param senderId A demo user ID or an unknown string.
 */
export function displaySenderName(senderId: string): string {
  return findDemoUser(senderId)?.displayName ?? senderId;
}
