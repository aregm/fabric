import { runApiHandler } from '@/chat/server/api-context';
import { handleHistory } from '@/chat/server/http';

/** `GET /api/messages`: one page of the message log. */
export function GET(request: Request): Promise<Response> {
  return runApiHandler((context) => handleHistory(request, context));
}
