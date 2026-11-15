/**
 * Attachment names and sizes (docs/paracels-chat.md, sections 10 and 12).
 * Pure functions, so the tests can load them under Node.
 */

const MAX_FILE_NAME_LENGTH = 200;

/**
 * A file name safe to store on disk and show in the chat. Path pieces are dropped.
 * @param fileName The name from the picker, which may be empty.
 */
export function sanitizeFileName(fileName: string): string {
  const base = fileName.split(/[/\\]/).pop() ?? '';
  const cleaned = base.replace(/[^\w.\- ()[\]]+/g, '_').replace(/^\.+/, '').slice(0, MAX_FILE_NAME_LENGTH);
  return cleaned.length > 0 ? cleaned : 'attachment';
}

/**
 * Whether the bubble should show the bytes as an image.
 * @param mimeType A MIME type, such as `image/jpeg`.
 */
export function isImageMime(mimeType: string): boolean {
  return mimeType.toLowerCase().startsWith('image/');
}

/**
 * A size for labels, using decimal units. 10 MB is 10,000,000 bytes (D5).
 * @param byteSize The size in bytes.
 */
export function formatByteSize(byteSize: number): string {
  if (!Number.isFinite(byteSize) || byteSize < 1000) {
    return `${Math.max(0, Math.round(byteSize))} bytes`;
  }
  if (byteSize < 1_000_000) {
    return `${Math.round(byteSize / 1000)} kilobytes`;
  }
  const megabytes = byteSize / 1_000_000;
  const rounded = megabytes >= 10 ? String(Math.round(megabytes)) : String(Math.round(megabytes * 10) / 10);
  return `${rounded} MB`;
}
