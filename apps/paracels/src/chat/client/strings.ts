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
  attach: 'Attach',
  attachHint: 'Add a photo or a file',
  photoLibrary: 'Photo Library',
  takePhoto: 'Take Photo',
  chooseFile: 'Choose File',
  removeAttachment: 'Remove attachment',
  removeAttachmentHint: 'Removes the attachment from this message',
  attachmentTooLarge: 'This attachment is over 10 MB, so it was not added.',
  photoLibraryDenied: 'Photo Library access is off. Allow it in Settings to send photos.',
  cameraDenied: 'Camera access is off. Allow it in Settings to take a photo.',
  cameraUnavailable: 'The camera is not available on this device.',
  attachmentUnreadable: 'That file could not be read.',
  attachmentNotSaved: 'This attachment could not be saved on this phone.',
  downloadFailed: 'Could not download this attachment.',
  previewFailed: 'Could not open this attachment.',
  downloading: (percent: number) => `Downloading ${percent}%`,
  openAttachment: 'Open attachment',
  openAttachmentHint: 'Downloads it and opens a preview',
} as const;

/**
 * Builds the VoiceOver label for one bubble.
 * @param sender The sender's display name.
 * @param time The formatted time.
 * @param status A short status, or `null` for someone else's message.
 * @param text The message text.
 */
export function bubbleAccessibilityLabel(
  sender: string,
  time: string,
  status: string | null,
  text: string,
  attachment: string | null = null,
): string {
  const statusPart = status === null ? '' : `, ${status}`;
  const textPart = text.length > 0 ? `: ${text}` : '';
  const attachmentPart = attachment === null ? '' : `. Attachment: ${attachment}`;
  return `${sender}, ${time}${statusPart}${textPart}${attachmentPart}`;
}
