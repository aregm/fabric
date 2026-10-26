/**
 * English UI strings (D25). Keys stay stable so a later i18n library can replace the values.
 */

export const strings = {
  chatTitle: 'Chat',
  connecting: 'Connecting…',
  unreachable: (address: string) => `Can't reach the chat server. Messages will wait on this phone. Trying ${address}.`,
  missingRoomKey: 'The room key is missing. Run npm run chat:setup, then restart the dev server.',
  emptyRoom: 'No messages yet. Say hello.',
  historyFailed: 'Could not load earlier messages.',
  retryHistory: 'Try again',
  composerPlaceholder: 'Message',
  send: 'Send',
  sendHint: 'Sends this message to the room',
  characterCount: (used: number, max: number) => `${used} of ${max} characters`,
  waiting: 'Waiting',
  sending: 'Sending',
  sent: 'Sent',
  notSent: 'Not sent',
  retry: 'Retry',
  retryHint: 'Sends this message again with the same ID',
  delete: 'Delete',
  deleteHint: 'Removes this unsent message from this phone',
  switchUser: 'Signed in as',
  switchUserHint: 'Choose who new messages are sent as',
  you: 'You',
  attachmentComing: 'Attachments arrive in a later step.',
} as const;

/**
 * Builds the VoiceOver label for one bubble.
 * @param sender The sender's display name.
 * @param time The formatted time.
 * @param status A short status, or `null` for someone else's message.
 * @param text The message text.
 */
export function bubbleAccessibilityLabel(sender: string, time: string, status: string | null, text: string): string {
  const statusPart = status === null ? '' : `, ${status}`;
  return `${sender}, ${time}${statusPart}: ${text}`;
}
