/**
 * One JSON line per server event (docs/paracels-chat.md, section 5.3). Fields
 * take only strings, numbers, booleans and null, so a payload object cannot be
 * logged by accident. Callers never pass message text, file contents or the room key.
 */

export type LogValue = string | number | boolean | null;

export type ChatLogger = (event: string, fields?: Readonly<Record<string, LogValue>>) => void;

/**
 * Creates a logger that writes each event as one JSON line, time first.
 * @param writeLine Receives each line, without a trailing newline.
 * @param now The clock; tests pass a fixed one.
 */
export function createJsonLogger(writeLine: (line: string) => void, now: () => Date = () => new Date()): ChatLogger {
  return (event, fields = {}) => {
    writeLine(JSON.stringify({ time: now().toISOString(), event, ...fields }));
  };
}
