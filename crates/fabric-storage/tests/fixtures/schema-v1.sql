-- Schema version 1 of the broker store: the normalized sqlite_schema of migration 1 (design 3.5, 3.6).
-- Frozen: a released migration never changes. Regenerate only with a new migration, never by editing.
-- One entry per schema object, ordered by type and name; SQL comments removed, whitespace collapsed,
-- and no space next to a parenthesis or a comma. Indexes SQLite makes itself have no statement.

-- index authorizations_one_live on authorizations
CREATE UNIQUE INDEX authorizations_one_live ON authorizations(request_id)WHERE state = 'issued';
-- index effects_provider_event on effects
CREATE UNIQUE INDEX effects_provider_event ON effects(account_id,calendar_id,provider_event_id)WHERE provider_event_id IS NOT NULL;
-- index grants_live_by_agent on grants
CREATE INDEX grants_live_by_agent ON grants(agent_id)WHERE ended_at_ms IS NULL;
-- index grants_live_by_session on grants
CREATE INDEX grants_live_by_session ON grants(session_id)WHERE ended_at_ms IS NULL;
-- index ledger_correlation on ledger
CREATE INDEX ledger_correlation ON ledger(correlation_id,seq)WHERE correlation_id IS NOT NULL;
-- index proof_reports_id on proof_reports
CREATE INDEX proof_reports_id ON proof_reports(proof_id);
-- index proof_reports_request on proof_reports
CREATE INDEX proof_reports_request ON proof_reports(request_id,proof_row);
-- index requests_due on requests
CREATE INDEX requests_due ON requests(due_at_ms)WHERE due_at_ms IS NOT NULL;
-- index requests_open_by_agent on requests
CREATE INDEX requests_open_by_agent ON requests(agent_id)WHERE terminal = 0;
-- index requests_received on requests
CREATE INDEX requests_received ON requests(agent_id,received_at_ms);
-- index sessions_one_live on sessions
CREATE UNIQUE INDEX sessions_one_live ON sessions(agent_id)WHERE ended_at_ms IS NULL;
-- index sqlite_autoindex_accounts_1 on accounts
-- index sqlite_autoindex_agents_1 on agents
-- index sqlite_autoindex_attempts_1 on attempts
-- index sqlite_autoindex_attempts_2 on attempts
-- index sqlite_autoindex_authorizations_1 on authorizations
-- index sqlite_autoindex_authorizations_2 on authorizations
-- index sqlite_autoindex_commands_1 on commands
-- index sqlite_autoindex_counters_1 on counters
-- index sqlite_autoindex_effects_1 on effects
-- index sqlite_autoindex_effects_2 on effects
-- index sqlite_autoindex_grants_1 on grants
-- index sqlite_autoindex_ledger_1 on ledger
-- index sqlite_autoindex_ledger_2 on ledger
-- index sqlite_autoindex_ledger_3 on ledger
-- index sqlite_autoindex_meta_1 on meta
-- index sqlite_autoindex_profile_versions_1 on profile_versions
-- index sqlite_autoindex_recipient_set_versions_1 on recipient_set_versions
-- index sqlite_autoindex_recipient_sets_1 on recipient_sets
-- index sqlite_autoindex_requests_1 on requests
-- index sqlite_autoindex_requests_2 on requests
-- index sqlite_autoindex_sessions_1 on sessions
-- table accounts on accounts
CREATE TABLE accounts(account_id TEXT PRIMARY KEY,revision INTEGER NOT NULL,state TEXT NOT NULL,record TEXT NOT NULL)STRICT;
-- table agents on agents
CREATE TABLE agents(agent_id TEXT PRIMARY KEY,revision INTEGER NOT NULL,paused INTEGER NOT NULL CHECK(paused IN(0,1)),record TEXT NOT NULL,enrolled_at_ms INTEGER NOT NULL)STRICT;
-- table attempts on attempts
CREATE TABLE attempts(effect_id TEXT NOT NULL REFERENCES effects,number INTEGER NOT NULL,authorization_id TEXT NOT NULL UNIQUE REFERENCES authorizations,record TEXT NOT NULL,PRIMARY KEY(effect_id,number))STRICT;
-- table audit on audit
CREATE TABLE audit(audit_id INTEGER PRIMARY KEY,at_ms INTEGER NOT NULL,run_id INTEGER,actor_kind TEXT NOT NULL CHECK(actor_kind IN('owner','agent','broker')),actor_agent_id TEXT,kind TEXT NOT NULL,request_id TEXT,detail TEXT NOT NULL)STRICT;
-- table authorizations on authorizations
CREATE TABLE authorizations(authorization_id TEXT PRIMARY KEY,request_id TEXT NOT NULL REFERENCES requests,attempt INTEGER NOT NULL,run_id INTEGER NOT NULL REFERENCES runs,record TEXT NOT NULL,fence TEXT NOT NULL,grant_ids TEXT NOT NULL,issued_at_ms INTEGER NOT NULL,expires_at_ms INTEGER NOT NULL,state TEXT NOT NULL CHECK(state IN('issued','consumed','voided')),consumed_at_ms INTEGER,voided_at_ms INTEGER,void_reason TEXT,CHECK(expires_at_ms = issued_at_ms + 60000),CHECK((state = 'consumed')=(consumed_at_ms IS NOT NULL)),CHECK((state = 'voided')=(voided_at_ms IS NOT NULL)),UNIQUE(request_id,attempt))STRICT;
-- table commands on commands
CREATE TABLE commands(command_id TEXT PRIMARY KEY,method TEXT NOT NULL,digest TEXT NOT NULL,answer TEXT NOT NULL)STRICT;
-- table counters on counters
CREATE TABLE counters(name TEXT PRIMARY KEY CHECK(name IN('pause_epoch','revocation_epoch')),value INTEGER NOT NULL CHECK(value >= 0))STRICT;
-- table effects on effects
CREATE TABLE effects(effect_id TEXT PRIMARY KEY,request_id TEXT NOT NULL UNIQUE REFERENCES requests,tool TEXT NOT NULL,account_id TEXT NOT NULL,calendar_id TEXT NOT NULL,action_digest TEXT NOT NULL,provider_event_id TEXT,created_at_ms INTEGER NOT NULL,attempts_started INTEGER NOT NULL DEFAULT 0,first_attempt_at_ms INTEGER)STRICT;
-- table grants on grants
CREATE TABLE grants(grant_id TEXT PRIMARY KEY,agent_id TEXT NOT NULL REFERENCES agents,revision INTEGER NOT NULL,term_kind TEXT NOT NULL CHECK(term_kind IN('one_request','session','until_revoked')),action_digest TEXT,session_id TEXT REFERENCES sessions,origin_request_id TEXT NOT NULL REFERENCES requests,record TEXT NOT NULL,ended_at_ms INTEGER,end_reason TEXT,CHECK((term_kind = 'one_request')=(action_digest IS NOT NULL)),CHECK((term_kind = 'session')=(session_id IS NOT NULL)),CHECK((ended_at_ms IS NULL)=(end_reason IS NULL)))STRICT;
-- table ledger on ledger
CREATE TABLE ledger(seq INTEGER PRIMARY KEY,operation_id TEXT NOT NULL UNIQUE,operation_type TEXT NOT NULL,payload_version INTEGER NOT NULL,committed_at_ms INTEGER NOT NULL,run_id INTEGER,actor_kind TEXT NOT NULL CHECK(actor_kind IN('owner','agent','broker')),actor_agent_id TEXT,idempotency_scope TEXT,idempotency_key TEXT,causation_id TEXT,correlation_id TEXT,payload TEXT NOT NULL,payload_hash TEXT NOT NULL,previous_hash TEXT NOT NULL,envelope_hash TEXT NOT NULL UNIQUE,CHECK((actor_kind = 'agent')=(actor_agent_id IS NOT NULL)),CHECK((idempotency_scope IS NULL)=(idempotency_key IS NULL)),UNIQUE(idempotency_scope,idempotency_key))STRICT;
-- table meta on meta
CREATE TABLE meta(key TEXT PRIMARY KEY,value TEXT NOT NULL)STRICT;
-- table preferences on preferences
CREATE TABLE preferences(id INTEGER PRIMARY KEY CHECK(id = 1),revision INTEGER NOT NULL,settings TEXT NOT NULL)STRICT;
-- table profile_state on profile_state
CREATE TABLE profile_state(id INTEGER PRIMARY KEY CHECK(id = 1),active_profile_id TEXT NOT NULL,active_version INTEGER NOT NULL,revision INTEGER NOT NULL)STRICT;
-- table profile_versions on profile_versions
CREATE TABLE profile_versions(profile_id TEXT NOT NULL,version INTEGER NOT NULL CHECK(version >= 1),record TEXT NOT NULL,created_at_ms INTEGER NOT NULL,PRIMARY KEY(profile_id,version))STRICT;
-- table proof_reports on proof_reports
CREATE TABLE proof_reports(proof_row INTEGER PRIMARY KEY,proof_id TEXT NOT NULL,request_id TEXT NOT NULL REFERENCES requests,purpose TEXT NOT NULL,prover_kind TEXT NOT NULL CHECK(prover_kind IN('isabelle','synthetic')),record TEXT NOT NULL,recorded_at_ms INTEGER NOT NULL)STRICT;
-- table recipient_set_versions on recipient_set_versions
CREATE TABLE recipient_set_versions(recipient_set_id TEXT NOT NULL REFERENCES recipient_sets,revision INTEGER NOT NULL,record TEXT NOT NULL,created_at_ms INTEGER NOT NULL,PRIMARY KEY(recipient_set_id,revision))STRICT;
-- table recipient_sets on recipient_sets
CREATE TABLE recipient_sets(recipient_set_id TEXT PRIMARY KEY,revision INTEGER NOT NULL,deleted_at_ms INTEGER)STRICT;
-- table requests on requests
CREATE TABLE requests(request_id TEXT PRIMARY KEY,agent_id TEXT NOT NULL REFERENCES agents,call_id TEXT NOT NULL,session_id TEXT REFERENCES sessions,account_id TEXT,state TEXT NOT NULL,terminal INTEGER NOT NULL CHECK(terminal IN(0,1)),revision INTEGER NOT NULL,received_at_ms INTEGER NOT NULL,record TEXT NOT NULL,action TEXT NOT NULL,scope TEXT,card TEXT,card_expires_at_ms INTEGER,approval TEXT,relied_grant_ids TEXT NOT NULL DEFAULT '[]',reproofs_classification INTEGER NOT NULL DEFAULT 0,reproofs_fresh_permit INTEGER NOT NULL DEFAULT 0,due_at_ms INTEGER,CHECK((state = 'pending_approval')<=(card IS NOT NULL)),UNIQUE(agent_id,call_id))STRICT;
-- table runs on runs
CREATE TABLE runs(run_id INTEGER PRIMARY KEY,started_at_ms INTEGER NOT NULL,ended_at_ms INTEGER,clean INTEGER NOT NULL DEFAULT 0 CHECK(clean IN(0,1)))STRICT;
-- table sessions on sessions
CREATE TABLE sessions(session_id TEXT PRIMARY KEY,agent_id TEXT NOT NULL REFERENCES agents,run_id INTEGER NOT NULL REFERENCES runs,started_at_ms INTEGER NOT NULL,ended_at_ms INTEGER,end_reason TEXT,CHECK((ended_at_ms IS NULL)=(end_reason IS NULL)))STRICT;
-- trigger audit_no_delete on audit
CREATE TRIGGER audit_no_delete BEFORE DELETE ON audit BEGIN SELECT RAISE(ABORT,'audit is append-only'); END;
-- trigger audit_no_update on audit
CREATE TRIGGER audit_no_update BEFORE UPDATE ON audit BEGIN SELECT RAISE(ABORT,'audit is append-only'); END;
-- trigger ledger_no_delete on ledger
CREATE TRIGGER ledger_no_delete BEFORE DELETE ON ledger BEGIN SELECT RAISE(ABORT,'ledger is append-only'); END;
-- trigger ledger_no_update on ledger
CREATE TRIGGER ledger_no_update BEFORE UPDATE ON ledger BEGIN SELECT RAISE(ABORT,'ledger is append-only'); END;
-- trigger proof_reports_no_delete on proof_reports
CREATE TRIGGER proof_reports_no_delete BEFORE DELETE ON proof_reports BEGIN SELECT RAISE(ABORT,'append-only'); END;
-- trigger proof_reports_no_update on proof_reports
CREATE TRIGGER proof_reports_no_update BEFORE UPDATE ON proof_reports BEGIN SELECT RAISE(ABORT,'append-only'); END;
