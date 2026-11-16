-- Schema version 1 of the broker store (CD-W04 design 3.5). Released migration SQL is frozen:
-- tests/fixtures/schema-v1.sql holds the schema it makes. The runner sets user_version,
-- meta.schema_version and the ledger row; this file creates the tables and seeds the projection.

CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL) STRICT;
  -- store_id, key_kind, schema_version, created_at_ms, notice_sequence, ledger_head_seq, ledger_head_hash
CREATE TABLE runs (run_id INTEGER PRIMARY KEY, started_at_ms INTEGER NOT NULL, ended_at_ms INTEGER,
  clean INTEGER NOT NULL DEFAULT 0 CHECK (clean IN (0,1))) STRICT;
CREATE TABLE counters (name TEXT PRIMARY KEY CHECK (name IN ('pause_epoch','revocation_epoch')),
  value INTEGER NOT NULL CHECK (value >= 0)) STRICT;

CREATE TABLE ledger (
  seq INTEGER PRIMARY KEY,
  operation_id TEXT NOT NULL UNIQUE,
  operation_type TEXT NOT NULL,
  payload_version INTEGER NOT NULL,
  committed_at_ms INTEGER NOT NULL,
  run_id INTEGER,
  actor_kind TEXT NOT NULL CHECK (actor_kind IN ('owner','agent','broker')),
  actor_agent_id TEXT,
  idempotency_scope TEXT, idempotency_key TEXT,
  causation_id TEXT, correlation_id TEXT,
  payload TEXT NOT NULL, payload_hash TEXT NOT NULL,
  previous_hash TEXT NOT NULL, envelope_hash TEXT NOT NULL UNIQUE,
  CHECK ((actor_kind = 'agent') = (actor_agent_id IS NOT NULL)),
  CHECK ((idempotency_scope IS NULL) = (idempotency_key IS NULL)),
  UNIQUE (idempotency_scope, idempotency_key)) STRICT;
CREATE INDEX ledger_correlation ON ledger(correlation_id, seq) WHERE correlation_id IS NOT NULL;
CREATE TRIGGER ledger_no_update BEFORE UPDATE ON ledger BEGIN SELECT RAISE(ABORT, 'ledger is append-only'); END;
CREATE TRIGGER ledger_no_delete BEFORE DELETE ON ledger BEGIN SELECT RAISE(ABORT, 'ledger is append-only'); END;

CREATE TABLE audit (audit_id INTEGER PRIMARY KEY, at_ms INTEGER NOT NULL, run_id INTEGER,
  actor_kind TEXT NOT NULL CHECK (actor_kind IN ('owner','agent','broker')), actor_agent_id TEXT,
  kind TEXT NOT NULL, request_id TEXT, detail TEXT NOT NULL) STRICT;
CREATE TRIGGER audit_no_update BEFORE UPDATE ON audit BEGIN SELECT RAISE(ABORT, 'audit is append-only'); END;
CREATE TRIGGER audit_no_delete BEFORE DELETE ON audit BEGIN SELECT RAISE(ABORT, 'audit is append-only'); END;

CREATE TABLE profile_versions (profile_id TEXT NOT NULL, version INTEGER NOT NULL CHECK (version >= 1),
  record TEXT NOT NULL, created_at_ms INTEGER NOT NULL, PRIMARY KEY (profile_id, version)) STRICT;
CREATE TABLE profile_state (id INTEGER PRIMARY KEY CHECK (id = 1), active_profile_id TEXT NOT NULL,
  active_version INTEGER NOT NULL, revision INTEGER NOT NULL) STRICT;          -- seeded: careful, revision 0
CREATE TABLE preferences (id INTEGER PRIMARY KEY CHECK (id = 1), revision INTEGER NOT NULL,
  settings TEXT NOT NULL) STRICT;                                              -- seeded: revision 0, empty (CD-Q11i)
CREATE TABLE recipient_sets (recipient_set_id TEXT PRIMARY KEY, revision INTEGER NOT NULL,
  deleted_at_ms INTEGER) STRICT;
CREATE TABLE recipient_set_versions (recipient_set_id TEXT NOT NULL REFERENCES recipient_sets,
  revision INTEGER NOT NULL, record TEXT NOT NULL, created_at_ms INTEGER NOT NULL,
  PRIMARY KEY (recipient_set_id, revision)) STRICT;

CREATE TABLE accounts (account_id TEXT PRIMARY KEY, revision INTEGER NOT NULL, state TEXT NOT NULL,
  record TEXT NOT NULL) STRICT;                                                -- contract Account (calendars inside)
CREATE TABLE agents (agent_id TEXT PRIMARY KEY, revision INTEGER NOT NULL,
  paused INTEGER NOT NULL CHECK (paused IN (0,1)), record TEXT NOT NULL,      -- AgentRow (no activity fields)
  enrolled_at_ms INTEGER NOT NULL) STRICT;
CREATE TABLE sessions (session_id TEXT PRIMARY KEY, agent_id TEXT NOT NULL REFERENCES agents,
  run_id INTEGER NOT NULL REFERENCES runs, started_at_ms INTEGER NOT NULL,     -- last activity: memory only (D35)
  ended_at_ms INTEGER, end_reason TEXT, CHECK ((ended_at_ms IS NULL) = (end_reason IS NULL))) STRICT;
CREATE UNIQUE INDEX sessions_one_live ON sessions(agent_id) WHERE ended_at_ms IS NULL;

CREATE TABLE requests (request_id TEXT PRIMARY KEY, agent_id TEXT NOT NULL REFERENCES agents,
  call_id TEXT NOT NULL, session_id TEXT REFERENCES sessions, account_id TEXT,
  state TEXT NOT NULL, terminal INTEGER NOT NULL CHECK (terminal IN (0,1)), revision INTEGER NOT NULL,
  received_at_ms INTEGER NOT NULL, record TEXT NOT NULL, action TEXT NOT NULL, scope TEXT,
  card TEXT, card_expires_at_ms INTEGER, approval TEXT,
  relied_grant_ids TEXT NOT NULL DEFAULT '[]',                                 -- JSON list of grant ids (D36)
  reproofs_classification INTEGER NOT NULL DEFAULT 0,
  reproofs_fresh_permit INTEGER NOT NULL DEFAULT 0,
  due_at_ms INTEGER,
  CHECK ((state = 'pending_approval') <= (card IS NOT NULL)),
  UNIQUE (agent_id, call_id)) STRICT;
CREATE INDEX requests_open_by_agent ON requests(agent_id) WHERE terminal = 0;
CREATE INDEX requests_received ON requests(agent_id, received_at_ms);
CREATE INDEX requests_due ON requests(due_at_ms) WHERE due_at_ms IS NOT NULL;

CREATE TABLE grants (grant_id TEXT PRIMARY KEY, agent_id TEXT NOT NULL REFERENCES agents,
  revision INTEGER NOT NULL,
  term_kind TEXT NOT NULL CHECK (term_kind IN ('one_request','session','until_revoked')),
  action_digest TEXT, session_id TEXT REFERENCES sessions,
  origin_request_id TEXT NOT NULL REFERENCES requests, record TEXT NOT NULL,
  ended_at_ms INTEGER, end_reason TEXT,
  CHECK ((term_kind = 'one_request') = (action_digest IS NOT NULL)),
  CHECK ((term_kind = 'session') = (session_id IS NOT NULL)),
  CHECK ((ended_at_ms IS NULL) = (end_reason IS NULL))) STRICT;
CREATE INDEX grants_live_by_agent ON grants(agent_id) WHERE ended_at_ms IS NULL;
CREATE INDEX grants_live_by_session ON grants(session_id) WHERE ended_at_ms IS NULL;

CREATE TABLE proof_reports (proof_row INTEGER PRIMARY KEY, proof_id TEXT NOT NULL,
  request_id TEXT NOT NULL REFERENCES requests, purpose TEXT NOT NULL,
  prover_kind TEXT NOT NULL CHECK (prover_kind IN ('isabelle','synthetic')),
  record TEXT NOT NULL, recorded_at_ms INTEGER NOT NULL) STRICT;              -- ProofRecord
CREATE INDEX proof_reports_id ON proof_reports(proof_id);                     -- not unique (THEORY 4 item 4)
CREATE INDEX proof_reports_request ON proof_reports(request_id, proof_row);
CREATE TRIGGER proof_reports_no_update BEFORE UPDATE ON proof_reports BEGIN SELECT RAISE(ABORT, 'append-only'); END;
CREATE TRIGGER proof_reports_no_delete BEFORE DELETE ON proof_reports BEGIN SELECT RAISE(ABORT, 'append-only'); END;

CREATE TABLE authorizations (authorization_id TEXT PRIMARY KEY, request_id TEXT NOT NULL REFERENCES requests,
  attempt INTEGER NOT NULL, run_id INTEGER NOT NULL REFERENCES runs, record TEXT NOT NULL, fence TEXT NOT NULL,
  grant_ids TEXT NOT NULL,                                                     -- the fresh proof's authorizing_grants
  issued_at_ms INTEGER NOT NULL, expires_at_ms INTEGER NOT NULL,
  state TEXT NOT NULL CHECK (state IN ('issued','consumed','voided')),
  consumed_at_ms INTEGER, voided_at_ms INTEGER, void_reason TEXT,
  CHECK (expires_at_ms = issued_at_ms + 60000),
  CHECK ((state = 'consumed') = (consumed_at_ms IS NOT NULL)),
  CHECK ((state = 'voided') = (voided_at_ms IS NOT NULL)),
  UNIQUE (request_id, attempt)) STRICT;
CREATE UNIQUE INDEX authorizations_one_live ON authorizations(request_id) WHERE state = 'issued';
CREATE TABLE effects (effect_id TEXT PRIMARY KEY, request_id TEXT NOT NULL UNIQUE REFERENCES requests,
  tool TEXT NOT NULL, account_id TEXT NOT NULL, calendar_id TEXT NOT NULL, action_digest TEXT NOT NULL,
  provider_event_id TEXT, created_at_ms INTEGER NOT NULL,
  attempts_started INTEGER NOT NULL DEFAULT 0, first_attempt_at_ms INTEGER) STRICT;
CREATE UNIQUE INDEX effects_provider_event ON effects(account_id, calendar_id, provider_event_id)
  WHERE provider_event_id IS NOT NULL;
CREATE TABLE attempts (effect_id TEXT NOT NULL REFERENCES effects, number INTEGER NOT NULL,
  authorization_id TEXT NOT NULL UNIQUE REFERENCES authorizations, record TEXT NOT NULL,   -- contract Attempt
  PRIMARY KEY (effect_id, number)) STRICT;

CREATE TABLE commands (command_id TEXT PRIMARY KEY, method TEXT NOT NULL, digest TEXT NOT NULL,
  answer TEXT NOT NULL) STRICT;

-- Seeds of the projection: the epochs at 0, the Careful profile active at revision 0, and empty
-- preferences at revision 0 (CD-Q11i).
INSERT INTO counters (name, value) VALUES ('pause_epoch', 0), ('revocation_epoch', 0);
INSERT INTO profile_state (id, active_profile_id, active_version, revision) VALUES (1, 'careful', 1, 0);
INSERT INTO preferences (id, revision, settings) VALUES (1, 0,
  '{"buffer":null,"protected_times":[],"uninterrupted":null,"warning_calendars":[],"workday":null,"zone":null}');
