import { runApiHandler } from '@/chat/server/api-context';
import { handleHealth } from '@/chat/server/http';

/** `GET /api/health`: checks that the chat server is reachable and the room key is right. */
export function GET(request: Request): Promise<Response> {
  return runApiHandler((context) => handleHealth(request, context));
}
