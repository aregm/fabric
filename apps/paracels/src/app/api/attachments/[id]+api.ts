import { runApiHandler } from '@/chat/server/api-context';
import { handleAttachmentDownload, handleAttachmentUpload } from '@/chat/server/http';

/** `PUT /api/attachments/{id}`: uploads one attachment's bytes. */
export function PUT(request: Request, { id }: Record<string, string>): Promise<Response> {
  return runApiHandler((context) => handleAttachmentUpload(request, id, context));
}

/** `GET /api/attachments/{id}`: downloads one attachment's bytes. */
export function GET(request: Request, { id }: Record<string, string>): Promise<Response> {
  return runApiHandler((context) => handleAttachmentDownload(request, id, context));
}
