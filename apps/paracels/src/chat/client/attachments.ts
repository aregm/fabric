/**
 * Picking, keeping, uploading and opening attachments (docs/paracels-chat.md, section 10).
 */

import * as DocumentPicker from 'expo-document-picker';
import { Directory, File, Paths, UploadType } from 'expo-file-system';
import * as ImagePicker from 'expo-image-picker';

import { CHAT_LIMITS } from '../shared/limits.ts';
import { UPLOADER_HEADER } from '../shared/protocol.ts';
import { isImageMime, sanitizeFileName } from './attachment-format.ts';
import { attachmentUrl, readUploadResponse, type UploadOutcome } from './api.ts';
import type { OutboxAttachment, OutboxItem } from './outbox.ts';

/** A file the composer is holding, still in the cache, not yet in the outbox. */
export type PickedAttachment = {
  uri: string;
  fileName: string;
  mimeType: string;
  byteSize: number;
  kind: 'image' | 'file';
};

export type PickFailureCode = 'canceled' | 'denied' | 'too_large' | 'unavailable' | 'unreadable';

export type PickResult = { ok: true; attachment: PickedAttachment } | { ok: false; code: PickFailureCode };

const IMAGE_QUALITY = 0.8;

/**
 * Photo Library. Images only, compressed at quality 0.8 (D21).
 */
export async function pickFromLibrary(): Promise<PickResult> {
  const permission = await ImagePicker.requestMediaLibraryPermissionsAsync();
  if (!permission.granted) {
    return { ok: false, code: 'denied' };
  }
  const result = await ImagePicker.launchImageLibraryAsync({ mediaTypes: 'images', quality: IMAGE_QUALITY });
  if (result.canceled || result.assets[0] === undefined) {
    return { ok: false, code: 'canceled' };
  }
  return pickedImage(result.assets[0]);
}

/**
 * Take Photo. The simulator has no camera, so this can come back `unavailable`.
 */
export async function takePhoto(): Promise<PickResult> {
  try {
    const permission = await ImagePicker.requestCameraPermissionsAsync();
    if (!permission.granted) {
      return { ok: false, code: 'denied' };
    }
    const result = await ImagePicker.launchCameraAsync({ mediaTypes: 'images', quality: IMAGE_QUALITY });
    if (result.canceled || result.assets[0] === undefined) {
      return { ok: false, code: 'canceled' };
    }
    return pickedImage(result.assets[0]);
  } catch {
    return { ok: false, code: 'unavailable' };
  }
}

/**
 * Choose File. The picker copies the file into the cache.
 */
export async function pickFile(): Promise<PickResult> {
  const result = await DocumentPicker.getDocumentAsync({ copyToCacheDirectory: true, multiple: false });
  if (result.canceled || result.assets[0] === undefined) {
    return { ok: false, code: 'canceled' };
  }
  const asset = result.assets[0];
  const byteSize = asset.size ?? fileByteSize(asset.uri);
  return finishPick({
    uri: asset.uri,
    fileName: sanitizeFileName(asset.name),
    mimeType: asset.mimeType ?? 'application/octet-stream',
    byteSize,
    kind: 'file',
  });
}

/**
 * Copies a picked file into the documents directory under `outbox/` (R11).
 * @param picked The file the composer is holding.
 * @param attachmentId A new UUID for this blob.
 * @returns The outbox attachment, or `null` when the copy fails.
 */
export function keepAttachmentCopy(picked: PickedAttachment, attachmentId: string): OutboxAttachment | null {
  try {
    const directory = new Directory(Paths.document, 'outbox');
    if (!directory.exists) {
      directory.create();
    }
    const destination = new File(directory, `${attachmentId}-${sanitizeFileName(picked.fileName)}`);
    new File(picked.uri).copy(destination);
    const byteSize = destination.exists ? destination.size : picked.byteSize;
    if (byteSize > CHAT_LIMITS.maxAttachmentBytes) {
      if (destination.exists) {
        destination.delete();
      }
      return null;
    }
    return {
      id: attachmentId,
      uri: destination.uri,
      fileName: picked.fileName,
      mimeType: picked.mimeType,
      byteSize,
      sha256: null,
    };
  } catch {
    return null;
  }
}

/**
 * Deletes the documents-directory copy. Missing files are ignored.
 * @param uri The copy's URI.
 */
export function deleteAttachmentCopy(uri: string): void {
  try {
    const file = new File(uri);
    if (file.exists) {
      file.delete();
    }
  } catch {
    // The row is already gone or about to be. A leftover file is not the message.
  }
}

/**
 * Uploads the outbox copy and records the size and hash the server returns.
 * @param item A sending message whose attachment has no SHA-256 yet.
 * @param roomKey The shared room key.
 * @param apiOrigin The API origin.
 */
export async function uploadOutboxAttachment(item: OutboxItem, roomKey: string, apiOrigin: string): Promise<UploadOutcome> {
  const attachment = item.attachment;
  if (attachment === null) {
    return { ok: false, retryable: false, code: 'upload_failed' };
  }
  let result: { status: number; body: string };
  try {
    const uploaded = await new File(attachment.uri).upload(attachmentUrl(apiOrigin, attachment.id), {
      httpMethod: 'PUT',
      uploadType: UploadType.BINARY_CONTENT,
      mimeType: attachment.mimeType,
      headers: {
        Authorization: `Bearer ${roomKey}`,
        [UPLOADER_HEADER]: item.senderId,
        'Content-Length': String(attachment.byteSize),
      },
    });
    result = { status: uploaded.status, body: uploaded.body };
  } catch {
    return { ok: false, retryable: true, code: 'network' };
  }
  return readUploadResponse(result.status, result.body);
}

export type OpenProgress = (fraction: number) => void;

/**
 * Downloads a stored attachment into the cache and opens Quick Look.
 * @param apiOrigin The API origin.
 * @param roomKey The shared room key.
 * @param attachmentId The blob id.
 * @param fileName The name to save under, sanitized here.
 * @param onProgress A fraction from 0 to 1 while the download runs.
 * @returns An error code, or `null` when Quick Look opened.
 */
export async function openRemoteAttachment(
  apiOrigin: string,
  roomKey: string,
  attachmentId: string,
  fileName: string,
  onProgress: OpenProgress,
): Promise<'download_failed' | 'preview_failed' | null> {
  const destination = new File(Paths.cache, sanitizeFileName(fileName));
  try {
    const task = File.createDownloadTask(attachmentUrl(apiOrigin, attachmentId), destination, {
      headers: { Authorization: `Bearer ${roomKey}` },
      onProgress: ({ bytesWritten, totalBytes }) => {
        if (totalBytes > 0) {
          onProgress(bytesWritten / totalBytes);
        }
      },
    });
    await task.downloadAsync();
  } catch {
    return 'download_failed';
  }
  return previewFile(destination, fileName);
}

/**
 * Opens a local copy in Quick Look.
 * @param uri The file URI.
 * @param fileName The title Quick Look shows.
 */
export async function openLocalAttachment(uri: string, fileName: string): Promise<'preview_failed' | null> {
  return previewFile(new File(uri), fileName);
}

async function previewFile(file: File, fileName: string): Promise<'preview_failed' | null> {
  try {
    if (!(await file.canPreview())) {
      return 'preview_failed';
    }
    await file.preview({ title: fileName });
    return null;
  } catch {
    return 'preview_failed';
  }
}

function pickedImage(asset: { uri: string; fileName?: string | null; mimeType?: string | null; fileSize?: number }): PickResult {
  return finishPick({
    uri: asset.uri,
    fileName: sanitizeFileName(asset.fileName ?? 'photo.jpg'),
    mimeType: asset.mimeType ?? 'image/jpeg',
    byteSize: asset.fileSize ?? fileByteSize(asset.uri),
    kind: 'image',
  });
}

function finishPick(attachment: Omit<PickedAttachment, 'byteSize' | 'kind'> & { byteSize: number | null; kind: PickedAttachment['kind'] }): PickResult {
  if (attachment.byteSize === null || !Number.isFinite(attachment.byteSize) || attachment.byteSize < 0) {
    return { ok: false, code: 'unreadable' };
  }
  if (attachment.byteSize > CHAT_LIMITS.maxAttachmentBytes) {
    return { ok: false, code: 'too_large' };
  }
  return {
    ok: true,
    attachment: {
      uri: attachment.uri,
      fileName: attachment.fileName,
      mimeType: attachment.mimeType,
      byteSize: attachment.byteSize,
      kind: isImageMime(attachment.mimeType) ? 'image' : 'file',
    },
  };
}

function fileByteSize(uri: string): number | null {
  try {
    const file = new File(uri);
    return file.exists ? file.size : null;
  } catch {
    return null;
  }
}
