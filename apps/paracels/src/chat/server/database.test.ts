import assert from 'node:assert/strict';
import { execFile } from 'node:child_process';
import { existsSync } from 'node:fs';
import { join } from 'node:path';
import { describe, it } from 'node:test';
import { promisify } from 'node:util';

import { isUuid } from '../shared/validate.ts';
import { openChatDatabase, SCHEMA_VERSION } from './database.ts';
import { createTemporaryDirectory } from './test-helpers.ts';

const runFile = promisify(execFile);

describe('openChatDatabase', () => {
  it('creates the schema with the settings of section 5.4', (t) => {
    const database = openChatDatabase(join(createTemporaryDirectory(t), 'nested', 'chat.sqlite'));
    t.after(() => database.connection.close());
    const readPragma = (name: string) => Object.values(database.connection.prepare(`PRAGMA ${name}`).get() ?? {})[0];

    assert.equal(readPragma('journal_mode'), 'wal');
    assert.equal(readPragma('synchronous'), 2, 'FULL');
    assert.equal(readPragma('busy_timeout'), 5000);
    assert.equal(readPragma('foreign_keys'), 1);
    assert.equal(readPragma('user_version'), SCHEMA_VERSION);
    const tables = database.connection
      .prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name")
      .all()
      .map((row) => row.name);
    assert.deepEqual(tables, ['attachments', 'messages', 'meta']);
    assert.ok(isUuid(database.databaseId));
  });

  it('keeps the database ID when reopened', (t) => {
    const filePath = join(createTemporaryDirectory(t), 'chat.sqlite');
    const first = openChatDatabase(filePath);
    first.connection.close();
    const second = openChatDatabase(filePath);
    t.after(() => second.connection.close());
    assert.equal(second.databaseId, first.databaseId);
  });

  it('migrates once when several processes open a new file at the same time', async (t) => {
    const filePath = join(createTemporaryDirectory(t), 'chat.sqlite');
    const moduleUrl = new URL('./database.ts', import.meta.url).href;
    const script = `
      const { openChatDatabase } = await import(${JSON.stringify(moduleUrl)});
      const database = openChatDatabase(${JSON.stringify(filePath)});
      process.stdout.write(database.databaseId);
      database.connection.close();
    `;
    const runs = Array.from({ length: 6 }, () =>
      runFile(process.execPath, ['--disable-warning=MODULE_TYPELESS_PACKAGE_JSON', '--input-type=module', '-e', script]),
    );
    const databaseIds = (await Promise.all(runs)).map((run) => run.stdout);
    assert.equal(new Set(databaseIds).size, 1);
    assert.ok(isUuid(databaseIds[0]));
  });

  it('refuses a schema newer than this code', (t) => {
    const filePath = join(createTemporaryDirectory(t), 'chat.sqlite');
    const database = openChatDatabase(filePath);
    database.connection.exec(`PRAGMA user_version = ${SCHEMA_VERSION + 1}`);
    database.connection.close();
    assert.throws(() => openChatDatabase(filePath), /schema version/);
  });

  it('opens read-only only when the database exists, and then refuses writes', (t) => {
    const filePath = join(createTemporaryDirectory(t), 'chat.sqlite');
    assert.throws(() => openChatDatabase(filePath, { readOnly: true }));
    assert.equal(existsSync(filePath), false);

    const writer = openChatDatabase(filePath);
    t.after(() => writer.connection.close());
    const reader = openChatDatabase(filePath, { readOnly: true });
    t.after(() => reader.connection.close());
    assert.equal(reader.databaseId, writer.databaseId);
    assert.throws(() => reader.connection.exec("INSERT INTO meta (key, value) VALUES ('x', 'y')"), /readonly/);
  });
});
