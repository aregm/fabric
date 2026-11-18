//! Applying a [`crate::Mutation`] to the projection: the only SQL that writes projection tables,
//! with the guards of design 9.1 (A).
//!
//! The same function applies a mutation inside a write and again during replay, so every guard
//! reads only the database it writes: never the clock, never memory.

use fabric_desktop_contracts::common::ToolName;
use fabric_desktop_contracts::lifecycle::RequestState;
use fabric_desktop_contracts::owner::{BuiltInProfile, GrantEnd, GrantTerm};
use fabric_desktop_contracts::scalar::{Id, Timestamp};
use rusqlite::{Connection, OptionalExtension as _, params};

use crate::mutation::Mutation;
use crate::read::{ReadTx, from_text, name_of, to_text};
use crate::rows::{AuthorizationState, Reproofs};
use crate::tx::{StoreFault, invariant, meta_value, set_meta, sql_fault};

/// Applies one mutation to the projection after checking its guards.
pub(crate) fn apply(connection: &Connection, mutation: &Mutation) -> Result<(), StoreFault> {
    let read = ReadTx::new(connection);
    match mutation {
        Mutation::StoreCreated {
            store_id,
            key_kind,
            created_at_ms,
        } => {
            if meta_value(connection, "store_id")?.is_some() {
                return Err(invariant("the store was already created"));
            }
            set_meta(connection, "store_id", store_id)?;
            set_meta(connection, "key_kind", key_kind.as_str())?;
            set_meta(connection, "created_at_ms", &created_at_ms.to_string())
        }
        Mutation::SchemaMigrated { from, to } => {
            if from >= to || user_version(connection)? != *to {
                return Err(invariant("a migration does not match the schema version"));
            }
            Ok(())
        }
        Mutation::RunStarted { run_id, at_ms } => {
            if read.last_run()?.is_some_and(|last| *run_id <= last) || *run_id < 1 {
                return Err(invariant("run ids rise"));
            }
            write(
                connection,
                "insert run",
                "INSERT INTO runs (run_id, started_at_ms) VALUES (?1, ?2)",
                params![run_id, at_ms],
            )
        }
        Mutation::RunEnded {
            run_id,
            at_ms,
            clean,
        } => {
            if read.current_run()? != Some(*run_id) {
                return Err(invariant("only the current run ends"));
            }
            write(
                connection,
                "end run",
                "UPDATE runs SET ended_at_ms = ?2, clean = ?3 WHERE run_id = ?1",
                params![run_id, at_ms, i64::from(*clean)],
            )
        }
        Mutation::CounterRaised { counter, to } => {
            let name = name_of(counter);
            let old: i64 = connection
                .query_row(
                    "SELECT value FROM counters WHERE name = ?1",
                    [&name],
                    |row| row.get(0),
                )
                .map_err(|e| sql_fault("read counter", &e))?;
            let to = signed(*to)?;
            if to < old {
                return Err(invariant("a counter never decreases"));
            }
            write(
                connection,
                "raise counter",
                "UPDATE counters SET value = ?2 WHERE name = ?1",
                params![name, to],
            )
        }
        Mutation::AgentPut { agent } => {
            let old = read.agent(&agent.agent_id)?.map(|a| a.revision);
            next_revision(old, agent.revision, "an agent's revision rises by one")?;
            write(
                connection,
                "put agent",
                "INSERT INTO agents (agent_id, revision, paused, record, enrolled_at_ms) \
                 VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT (agent_id) DO UPDATE SET \
                 revision = excluded.revision, paused = excluded.paused, \
                 record = excluded.record, enrolled_at_ms = excluded.enrolled_at_ms",
                params![
                    agent.agent_id.as_str(),
                    signed(agent.revision)?,
                    i64::from(agent.paused),
                    to_text(agent),
                    agent.enrolled_at_ms
                ],
            )
        }
        Mutation::SessionStarted { session } => {
            if session.ended.is_some() {
                return Err(invariant("a session starts live"));
            }
            if read.current_run()? != Some(session.run_id) {
                return Err(invariant("a session starts in the current run"));
            }
            if read.agent(&session.agent_id)?.is_none() {
                return Err(invariant("a session belongs to a known agent"));
            }
            write(
                connection,
                "start session",
                "INSERT INTO sessions (session_id, agent_id, run_id, started_at_ms) \
                 VALUES (?1, ?2, ?3, ?4)",
                params![
                    session.session_id.as_str(),
                    session.agent_id.as_str(),
                    session.run_id,
                    session.started_at_ms
                ],
            )
        }
        Mutation::SessionEnded {
            session_id,
            at_ms,
            reason,
        } => {
            match read.session(session_id)? {
                Some(session) if session.ended.is_none() => {}
                _ => return Err(invariant("only a live session ends")),
            }
            write(
                connection,
                "end session",
                "UPDATE sessions SET ended_at_ms = ?2, end_reason = ?3 WHERE session_id = ?1",
                params![session_id.as_str(), at_ms, name_of(reason)],
            )
        }
        Mutation::ProfileVersionAdded { profile, at_ms } => {
            let latest: Option<i64> = connection
                .query_row(
                    "SELECT max(version) FROM profile_versions WHERE profile_id = ?1",
                    [profile.profile_id.as_str()],
                    |row| row.get(0),
                )
                .map_err(|e| sql_fault("read profiles", &e))?;
            let expected = latest.unwrap_or(0) + 1;
            if signed(profile.version.get())? != expected {
                return Err(invariant("a profile's version rises by one"));
            }
            if BuiltInProfile::ALL
                .iter()
                .any(|b| b.profile_id() == profile.profile_id.as_str())
            {
                return Err(invariant("a custom profile never takes a built-in id"));
            }
            write(
                connection,
                "add profile version",
                "INSERT INTO profile_versions (profile_id, version, record, created_at_ms) \
                 VALUES (?1, ?2, ?3, ?4)",
                params![
                    profile.profile_id.as_str(),
                    expected,
                    to_text(profile),
                    at_ms
                ],
            )
        }
        Mutation::ProfileStateSet {
            active_profile_id,
            active_version,
            revision,
        } => {
            let old: i64 = connection
                .query_row(
                    "SELECT revision FROM profile_state WHERE id = 1",
                    [],
                    |row| row.get(0),
                )
                .map_err(|e| sql_fault("read profile state", &e))?;
            if signed(*revision)? != old + 1 {
                return Err(invariant("the profiles revision rises by one"));
            }
            let exists = if BuiltInProfile::ALL
                .iter()
                .any(|b| b.profile_id() == active_profile_id.as_str())
            {
                *active_version == 1
            } else {
                connection
                    .query_row(
                        "SELECT 1 FROM profile_versions WHERE profile_id = ?1 AND version = ?2",
                        params![active_profile_id.as_str(), signed(*active_version)?],
                        |_| Ok(()),
                    )
                    .optional()
                    .map_err(|e| sql_fault("read profiles", &e))?
                    .is_some()
            };
            if !exists {
                return Err(invariant("the active profile exists"));
            }
            write(
                connection,
                "set profile state",
                "UPDATE profile_state SET active_profile_id = ?1, active_version = ?2, \
                 revision = ?3 WHERE id = 1",
                params![
                    active_profile_id.as_str(),
                    signed(*active_version)?,
                    signed(*revision)?
                ],
            )
        }
        Mutation::PreferencesPut { preferences } => {
            let old = read.preferences()?.revision.get();
            if preferences.revision.get() != old + 1 {
                return Err(invariant("the preferences revision rises by one"));
            }
            write(
                connection,
                "put preferences",
                "UPDATE preferences SET revision = ?1, settings = ?2 WHERE id = 1",
                params![
                    signed(preferences.revision.get())?,
                    to_text(&preferences.settings)
                ],
            )
        }
        Mutation::RecipientSetVersionAdded { set, at_ms } => {
            let id = set.recipient_set_id.as_str();
            let old: Option<(i64, Option<i64>)> = connection
                .query_row(
                    "SELECT revision, deleted_at_ms FROM recipient_sets WHERE recipient_set_id = ?1",
                    [id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()
                .map_err(|e| sql_fault("read recipient sets", &e))?;
            let revision = signed(set.revision.get())?;
            match old {
                None if revision == 1 => write(
                    connection,
                    "add recipient set",
                    "INSERT INTO recipient_sets (recipient_set_id, revision) VALUES (?1, ?2)",
                    params![id, revision],
                )?,
                Some((old, None)) if revision == old + 1 => write(
                    connection,
                    "change recipient set",
                    "UPDATE recipient_sets SET revision = ?2 WHERE recipient_set_id = ?1",
                    params![id, revision],
                )?,
                Some((_, Some(_))) => {
                    return Err(invariant("a deleted recipient set stays deleted"));
                }
                _ => return Err(invariant("a recipient set's revision rises by one")),
            }
            write(
                connection,
                "add recipient set version",
                "INSERT INTO recipient_set_versions (recipient_set_id, revision, record, created_at_ms) \
                 VALUES (?1, ?2, ?3, ?4)",
                params![id, revision, to_text(set), at_ms],
            )
        }
        Mutation::RecipientSetDeleted {
            recipient_set_id,
            at_ms,
        } => {
            let deleted: Option<Option<i64>> = connection
                .query_row(
                    "SELECT deleted_at_ms FROM recipient_sets WHERE recipient_set_id = ?1",
                    [recipient_set_id.as_str()],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|e| sql_fault("read recipient sets", &e))?;
            if deleted != Some(None) {
                return Err(invariant("only a current recipient set is deleted"));
            }
            write(
                connection,
                "delete recipient set",
                "UPDATE recipient_sets SET deleted_at_ms = ?2 WHERE recipient_set_id = ?1",
                params![recipient_set_id.as_str(), at_ms],
            )
        }
        Mutation::AccountPut { account } => {
            let old = read.account(&account.account_id)?.map(|a| a.revision.get());
            next_revision(
                old,
                account.revision.get(),
                "an account's revision rises by one",
            )?;
            write(
                connection,
                "put account",
                "INSERT INTO accounts (account_id, revision, state, record) VALUES (?1, ?2, ?3, ?4) \
                 ON CONFLICT (account_id) DO UPDATE SET revision = excluded.revision, \
                 state = excluded.state, record = excluded.record",
                params![
                    account.account_id.as_str(),
                    signed(account.revision.get())?,
                    name_of(&account.state),
                    to_text(account)
                ],
            )
        }
        Mutation::RequestInserted { row, due_at_ms } => {
            let request = &row.request;
            let state = request.status.state();
            if state != RequestState::INITIAL || request.revision.get() != 1 {
                return Err(invariant("a request starts received at revision 1"));
            }
            if row.action.tool() != request.tool {
                return Err(invariant("a request's tool is its action's"));
            }
            write(
                connection,
                "insert request",
                "INSERT INTO requests (request_id, agent_id, call_id, session_id, account_id, \
                 state, terminal, revision, received_at_ms, record, action, scope, due_at_ms) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
                params![
                    request.request_id.as_str(),
                    request.agent_id.as_str(),
                    row.call_id.as_str(),
                    row.session_id.as_ref().map(Id::as_str),
                    request.account_id.as_ref().map(Id::as_str),
                    state.as_str(),
                    i64::from(state.is_terminal()),
                    1_i64,
                    millis(&request.received_at)?,
                    to_text(request),
                    to_text(&row.action),
                    row.scope.as_ref().map(to_text),
                    due_at_ms
                ],
            )
        }
        Mutation::RequestMoved {
            request_id,
            revision,
            status,
            at_ms,
            due_at_ms,
            // The cause lives in the ledger only; `WriteTx::apply` checks it, replay does not.
            undecided_cause: _,
        } => {
            let mut request = known_request(&read, request_id)?.row.request;
            let from = request.status.state();
            let to = status.state();
            if !from.can_transition_to(to) {
                return Err(invariant("a request moves only along the lifecycle"));
            }
            if *revision != request.revision.get() + 1 {
                return Err(invariant("a request's revision rises by one"));
            }
            request.revision = fabric_desktop_contracts::scalar::PositiveCount::new(*revision)
                .map_err(|_| invariant("a request's revision rises by one"))?;
            request.status = status.clone();
            request.updated_at = timestamp(*at_ms)?;
            write(
                connection,
                "move request",
                "UPDATE requests SET state = ?2, terminal = ?3, revision = ?4, record = ?5, \
                 due_at_ms = ?6 WHERE request_id = ?1",
                params![
                    request_id.as_str(),
                    to.as_str(),
                    i64::from(to.is_terminal()),
                    signed(*revision)?,
                    to_text(&request),
                    due_at_ms
                ],
            )
        }
        Mutation::RequestScheduled {
            request_id,
            due_at_ms,
            reproofs,
        } => {
            let old = known_request(&read, request_id)?.reproofs;
            if !never_lower(old, *reproofs) {
                return Err(invariant("re-proof counts never decrease"));
            }
            write(
                connection,
                "schedule request",
                "UPDATE requests SET due_at_ms = ?2, reproofs_classification = ?3, \
                 reproofs_fresh_permit = ?4 WHERE request_id = ?1",
                params![
                    request_id.as_str(),
                    due_at_ms,
                    i64::from(reproofs.classification),
                    i64::from(reproofs.fresh_permit)
                ],
            )
        }
        Mutation::CardOpened {
            request_id,
            card,
            opened_at_ms,
        } => {
            let record = known_request(&read, request_id)?;
            if record.row.request.status.state().is_terminal() {
                return Err(invariant("a card opens only for an open request"));
            }
            let expires = card
                .expires_at
                .unix_millis()
                .ok_or(invariant("a card expires at a real instant"))?;
            if expires <= *opened_at_ms {
                return Err(invariant("a card expires after it opens"));
            }
            write(
                connection,
                "open card",
                "UPDATE requests SET card = ?2, card_expires_at_ms = ?3 WHERE request_id = ?1",
                params![request_id.as_str(), to_text(card), expires],
            )
        }
        Mutation::ApprovalRecorded {
            request_id,
            approval,
        } => {
            let record = known_request(&read, request_id)?;
            if record.row.request.status.state() != RequestState::PendingApproval {
                return Err(invariant("only a pending request is decided"));
            }
            write(
                connection,
                "record approval",
                "UPDATE requests SET approval = ?2 WHERE request_id = ?1",
                params![request_id.as_str(), to_text(approval)],
            )
        }
        Mutation::ReliedGrantSet {
            request_id,
            grant_ids,
        } => {
            // The one check of D36's rule; the core's grant check (5.7) decides the request
            // state before it, so reaching this refusal is a broken rule.
            let agent = known_request(&read, request_id)?.row.request.agent_id;
            grants_of(
                &read,
                &agent,
                grant_ids,
                "a relied grant belongs to the request's agent",
            )?;
            write(
                connection,
                "set relied grants",
                "UPDATE requests SET relied_grant_ids = ?2 WHERE request_id = ?1",
                params![request_id.as_str(), to_text(grant_ids)],
            )
        }
        Mutation::ProofRecorded { proof, at_ms } => {
            let report = &proof.report;
            known_request(&read, &report.request_id)?;
            write(
                connection,
                "record proof",
                "INSERT INTO proof_reports (proof_id, request_id, purpose, prover_kind, record, \
                 recorded_at_ms) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    report.proof_id.as_str(),
                    report.request_id.as_str(),
                    name_of(&report.purpose),
                    name_of(&proof.prover),
                    to_text(proof),
                    at_ms
                ],
            )
        }
        Mutation::GrantInserted { grant } => {
            if grant.revision.get() != 1 || grant.ended.is_some() {
                return Err(invariant("a grant starts live at revision 1"));
            }
            let Some(origin_request_id) = grant.origin.request_id() else {
                return Err(invariant(
                    "a grant made by an enrollment approval is not stored by this version of the store",
                ));
            };
            let origin = known_request(&read, origin_request_id)?;
            if origin.row.request.agent_id != grant.agent_id {
                return Err(invariant("a grant belongs to its origin request's agent"));
            }
            let (kind, action_digest, session_id) = match &grant.term {
                GrantTerm::OneRequest { action_digest } => {
                    ("one_request", Some(action_digest.as_str()), None)
                }
                GrantTerm::Session { session_id } => ("session", None, Some(session_id.as_str())),
                GrantTerm::UntilRevoked => ("until_revoked", None, None),
            };
            write(
                connection,
                "insert grant",
                "INSERT INTO grants (grant_id, agent_id, revision, term_kind, action_digest, \
                 session_id, origin_request_id, record) VALUES (?1, ?2, 1, ?3, ?4, ?5, ?6, ?7)",
                params![
                    grant.grant_id.as_str(),
                    grant.agent_id.as_str(),
                    kind,
                    action_digest,
                    session_id,
                    origin_request_id.as_str(),
                    to_text(grant)
                ],
            )
        }
        Mutation::GrantEnded {
            grant_id,
            revision,
            end,
        } => {
            let mut grant = read
                .grant(grant_id)?
                .ok_or(invariant("only a known grant ends"))?;
            if grant.ended.is_some() {
                return Err(invariant("an ended grant never changes"));
            }
            if *revision != grant.revision.get() + 1 {
                return Err(invariant("a grant's revision rises by one"));
            }
            grant.revision = fabric_desktop_contracts::scalar::PositiveCount::new(*revision)
                .map_err(|_| invariant("a grant's revision rises by one"))?;
            grant.ended = Some(GrantEnd {
                at: end.at.clone(),
                reason: end.reason,
            });
            write(
                connection,
                "end grant",
                "UPDATE grants SET revision = ?2, record = ?3, ended_at_ms = ?4, end_reason = ?5 \
                 WHERE grant_id = ?1",
                params![
                    grant_id.as_str(),
                    signed(*revision)?,
                    to_text(&grant),
                    millis(&end.at)?,
                    name_of(&end.reason)
                ],
            )
        }
        Mutation::AuthorizationIssued { record } => {
            let authorization = &record.authorization;
            if record.state != AuthorizationState::Issued || authorization.consumed_at.is_some() {
                return Err(invariant("an authorization is issued unused"));
            }
            if read.current_run()? != Some(record.run_id) {
                return Err(invariant("an authorization is issued in the current run"));
            }
            if record.grant_ids.first() != Some(&authorization.grant_id) {
                return Err(invariant("an authorization names its first grant"));
            }
            if !authorization.expiry_matches_lifetime() {
                return Err(invariant("an authorization lives 60 s"));
            }
            let request = known_request(&read, &authorization.request_id)?;
            if request.row.request.agent_id != authorization.agent_id {
                return Err(invariant("an authorization is for its request's agent"));
            }
            grants_of(
                &read,
                &authorization.agent_id,
                &record.grant_ids,
                "an authorization rests on grants of its agent",
            )?;
            write(
                connection,
                "issue authorization",
                "INSERT INTO authorizations (authorization_id, request_id, attempt, run_id, record, \
                 fence, grant_ids, issued_at_ms, expires_at_ms, state) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'issued')",
                params![
                    authorization.authorization_id.as_str(),
                    authorization.request_id.as_str(),
                    i64::from(record.attempt),
                    record.run_id,
                    to_text(authorization),
                    to_text(&record.fence),
                    to_text(&record.grant_ids),
                    millis(&authorization.issued_at)?,
                    millis(&authorization.expires_at)?
                ],
            )
        }
        Mutation::AuthorizationConsumed {
            authorization_id,
            at_ms,
        } => {
            let (mut record, expires_at_ms) = authorization_of(connection, authorization_id)?;
            match record.state {
                AuthorizationState::Issued => {}
                AuthorizationState::Consumed => {
                    return Err(invariant("an authorization is consumed once"));
                }
                AuthorizationState::Voided => {
                    return Err(invariant("a voided authorization is never consumed"));
                }
            }
            if *at_ms >= expires_at_ms {
                return Err(invariant("an expired authorization is never consumed"));
            }
            if read.current_run()? != Some(record.run_id) {
                return Err(invariant(
                    "an authorization of another run is never consumed",
                ));
            }
            record.authorization.consumed_at = Some(timestamp(*at_ms)?);
            write(
                connection,
                "consume authorization",
                "UPDATE authorizations SET state = 'consumed', consumed_at_ms = ?2, record = ?3 \
                 WHERE authorization_id = ?1",
                params![
                    authorization_id.as_str(),
                    at_ms,
                    to_text(&record.authorization)
                ],
            )
        }
        Mutation::AuthorizationVoided {
            authorization_id,
            at_ms,
            reason,
        } => {
            let (record, _) = authorization_of(connection, authorization_id)?;
            if record.state != AuthorizationState::Issued {
                return Err(invariant("only an unused authorization is voided"));
            }
            write(
                connection,
                "void authorization",
                "UPDATE authorizations SET state = 'voided', voided_at_ms = ?2, void_reason = ?3 \
                 WHERE authorization_id = ?1",
                params![authorization_id.as_str(), at_ms, name_of(reason)],
            )
        }
        Mutation::EffectReserved { effect } => {
            let request = known_request(&read, &effect.request_id)?;
            if request.row.action.tool() != effect.tool {
                return Err(invariant("an effect's tool is its request's"));
            }
            let create = effect.tool == ToolName::ProposeCreate;
            if create != effect.provider_event_id.is_some() {
                return Err(invariant("only a create reserves its event id, always"));
            }
            if effect.attempts_started != 0 || effect.first_attempt_at_ms.is_some() {
                return Err(invariant("an effect is reserved before its first attempt"));
            }
            write(
                connection,
                "reserve effect",
                "INSERT INTO effects (effect_id, request_id, tool, account_id, calendar_id, \
                 action_digest, provider_event_id, created_at_ms) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    effect.effect_id.as_str(),
                    effect.request_id.as_str(),
                    effect.tool.as_str(),
                    effect.account_id.as_str(),
                    effect.calendar_id.as_str(),
                    effect.action_digest.as_str(),
                    effect.provider_event_id,
                    effect.created_at_ms
                ],
            )
        }
        Mutation::DispatchIntent { attempt } => {
            let effect = read
                .effect(&attempt.effect_id)?
                .ok_or(invariant("an attempt belongs to a reserved effect"))?;
            let (authorization, _) = authorization_of(connection, &attempt.authorization_id)?;
            if authorization.state != AuthorizationState::Consumed {
                return Err(invariant(
                    "an attempt follows the consumption of its authorization",
                ));
            }
            if authorization.authorization.request_id != effect.request_id {
                return Err(invariant("an attempt's authorization is for its request"));
            }
            let number = attempt.attempt.number.get();
            if number != u64::from(effect.attempts_started) + 1 {
                return Err(invariant("attempts are numbered from one, in order"));
            }
            if attempt.attempt.finished_at.is_some() || attempt.attempt.outcome.is_some() {
                return Err(invariant("an attempt starts unfinished"));
            }
            let started = millis(&attempt.attempt.started_at)?;
            write(
                connection,
                "insert attempt",
                "INSERT INTO attempts (effect_id, number, authorization_id, record) \
                 VALUES (?1, ?2, ?3, ?4)",
                params![
                    attempt.effect_id.as_str(),
                    signed(number)?,
                    attempt.authorization_id.as_str(),
                    to_text(&attempt.attempt)
                ],
            )?;
            write(
                connection,
                "start attempt",
                "UPDATE effects SET attempts_started = attempts_started + 1, \
                 first_attempt_at_ms = coalesce(first_attempt_at_ms, ?2) WHERE effect_id = ?1",
                params![attempt.effect_id.as_str(), started],
            )
        }
        Mutation::AttemptFinished {
            effect_id,
            number,
            finished_at_ms,
            outcome,
        } => {
            let record: Option<String> = connection
                .query_row(
                    "SELECT record FROM attempts WHERE effect_id = ?1 AND number = ?2",
                    params![effect_id.as_str(), i64::from(*number)],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|e| sql_fault("read attempts", &e))?;
            let record = record.ok_or(invariant("only a started attempt finishes"))?;
            let mut attempt: fabric_desktop_contracts::owner::Attempt =
                from_text(&record, "attempts", "record")?;
            if attempt.finished_at.is_some() || attempt.outcome.is_some() {
                return Err(invariant("an attempt finishes once"));
            }
            attempt.finished_at = Some(timestamp(*finished_at_ms)?);
            attempt.outcome = Some(*outcome);
            write(
                connection,
                "finish attempt",
                "UPDATE attempts SET record = ?3 WHERE effect_id = ?1 AND number = ?2",
                params![effect_id.as_str(), i64::from(*number), to_text(&attempt)],
            )
        }
        Mutation::CommandAnswered { row } => write(
            connection,
            "answer command",
            "INSERT INTO commands (command_id, method, digest, answer) VALUES (?1, ?2, ?3, ?4)",
            params![
                row.command_id.as_str(),
                row.method,
                row.digest.as_str(),
                row.answer
            ],
        ),
    }
}

/// Runs one projection write.
fn write<P: rusqlite::Params>(
    connection: &Connection,
    op: &'static str,
    sql: &str,
    params: P,
) -> Result<(), StoreFault> {
    let mut statement = connection
        .prepare_cached(sql)
        .map_err(|e| sql_fault(op, &e))?;
    statement.execute(params).map_err(|e| sql_fault(op, &e))?;
    Ok(())
}

/// The schema version of the database.
pub(crate) fn user_version(connection: &Connection) -> Result<u32, StoreFault> {
    let version: i64 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(|e| sql_fault("read user_version", &e))?;
    u32::try_from(version).map_err(|_| invariant("the schema version is a small count"))
}

/// A count as SQLite stores it.
fn signed(value: u64) -> Result<i64, StoreFault> {
    i64::try_from(value).map_err(|_| invariant("a count fits in 63 bits"))
}

/// A new record starts at revision 1; a changed one rises by one.
fn next_revision(old: Option<u64>, new: u64, what: &'static str) -> Result<(), StoreFault> {
    if new == old.unwrap_or(0) + 1 {
        Ok(())
    } else {
        Err(invariant(what))
    }
}

/// Whether each re-proof count stayed or rose.
fn never_lower(old: Reproofs, new: Reproofs) -> bool {
    new.classification >= old.classification && new.fresh_permit >= old.fresh_permit
}

/// The Unix milliseconds of a contract timestamp.
fn millis(timestamp: &Timestamp) -> Result<i64, StoreFault> {
    timestamp
        .unix_millis()
        .ok_or(invariant("a timestamp names a real instant"))
}

/// The contract timestamp of Unix milliseconds.
fn timestamp(ms: i64) -> Result<Timestamp, StoreFault> {
    let ms = u64::try_from(ms).map_err(|_| invariant("a time is after 1970"))?;
    if ms > crate::time::MAX_TIMESTAMP_MS {
        return Err(invariant("a time is before the year 10000"));
    }
    Ok(crate::time::format_timestamp(ms))
}

/// A request that must exist.
fn known_request(
    read: &ReadTx<'_>,
    request_id: &Id,
) -> Result<crate::rows::RequestRecord, StoreFault> {
    read.request(request_id)?
        .ok_or(invariant("the request exists"))
}

/// Checks that every id names a grant of `agent_id`.
fn grants_of(
    read: &ReadTx<'_>,
    agent_id: &Id,
    grant_ids: &[Id],
    what: &'static str,
) -> Result<(), StoreFault> {
    for grant_id in grant_ids {
        if read.grant_agent(grant_id)?.as_deref() != Some(agent_id.as_str()) {
            return Err(invariant(what));
        }
    }
    Ok(())
}

/// An authorization with its expiry in milliseconds.
fn authorization_of(
    connection: &Connection,
    authorization_id: &Id,
) -> Result<(crate::rows::AuthorizationRecord, i64), StoreFault> {
    let read = ReadTx::new(connection);
    let record = read
        .authorization(authorization_id)?
        .ok_or(invariant("the authorization exists"))?;
    let expires = millis(&record.authorization.expires_at)?;
    Ok((record, expires))
}

#[cfg(test)]
mod tests {
    use fabric_desktop_contracts::owner::{GrantEndReason, LifecycleStatus};

    use super::*;
    use crate::mutation::{Counter, SessionEndReason, VoidReason};
    use crate::rows::{EffectRow, SessionRow};
    use crate::testing::{self, T0, TempDir, id, refused_with};
    use crate::{Store, TxError};

    const CREATE: &str = "fabric_calendar_propose_create";

    fn apply(store: &mut Store, mutations: Vec<Mutation>) -> Result<(), &'static str> {
        match testing::apply_all(store, T0, mutations) {
            Ok(_) => Ok(()),
            result => Err(refused_with(result)),
        }
    }

    fn moved(request: &str, revision: u64, state: &str) -> Mutation {
        Mutation::RequestMoved {
            request_id: id(request),
            revision,
            status: testing::status(state),
            at_ms: T0,
            due_at_ms: Some(T0),
            undecided_cause: None,
        }
    }

    fn ended(grant: &str, revision: u64) -> Mutation {
        Mutation::GrantEnded {
            grant_id: id(grant),
            revision,
            end: GrantEnd {
                at: testing::ts(T0),
                reason: GrantEndReason::RevokedByOwner,
            },
        }
    }

    #[test]
    fn a_request_moves_only_along_the_lifecycle() {
        let dir = TempDir::new();
        let mut store = testing::world(&dir);
        assert_eq!(
            apply(&mut store, vec![moved("request-1", 2, "queued")]),
            Err("a request moves only along the lifecycle")
        );
        apply(&mut store, vec![moved("request-1", 2, "validating")]).unwrap();
        let record = store
            .read(|r| r.request(&id("request-1")))
            .unwrap()
            .unwrap();
        assert_eq!(record.row.request.status, testing::status("validating"));
        assert_eq!(record.row.request.revision.get(), 2);
        store.verify_replay().unwrap();
    }

    #[test]
    fn a_revision_never_skips() {
        let dir = TempDir::new();
        let mut store = testing::world(&dir);
        assert_eq!(
            apply(&mut store, vec![moved("request-1", 3, "validating")]),
            Err("a request's revision rises by one")
        );
        assert_eq!(
            apply(&mut store, vec![ended("grant-1", 3)]),
            Err("a grant's revision rises by one")
        );
        assert_eq!(
            apply(
                &mut store,
                vec![Mutation::AgentPut {
                    agent: testing::agent("agent-1", 3)
                }]
            ),
            Err("an agent's revision rises by one")
        );
        assert_eq!(
            apply(
                &mut store,
                vec![Mutation::AccountPut {
                    account: testing::account("account-2", 2)
                }]
            ),
            Err("an account's revision rises by one")
        );
    }

    #[test]
    fn an_ended_grant_never_changes() {
        let dir = TempDir::new();
        let mut store = testing::world(&dir);
        apply(&mut store, vec![ended("grant-1", 2)]).unwrap();
        assert_eq!(
            apply(&mut store, vec![ended("grant-1", 3)]),
            Err("an ended grant never changes")
        );
        let grant = store.read(|r| r.grant(&id("grant-1"))).unwrap().unwrap();
        assert_eq!(grant.revision.get(), 2);
        assert!(store.read(|r| r.live_grants()).unwrap().len() == 1);
    }

    /// The world with `request-1` queued under an authorization issued at `T0` in run 1.
    fn queued(dir: &TempDir) -> Store {
        let mut store = testing::world(dir);
        apply(
            &mut store,
            vec![
                Mutation::AuthorizationIssued {
                    record: testing::authorization(
                        "auth-1",
                        "request-1",
                        "agent-1",
                        "grant-1",
                        1,
                        T0,
                    ),
                },
                Mutation::EffectReserved {
                    effect: testing::effect("effect-1", "request-1", CREATE),
                },
            ],
        )
        .unwrap();
        store
    }

    fn consumed(at_ms: i64) -> Mutation {
        Mutation::AuthorizationConsumed {
            authorization_id: id("auth-1"),
            at_ms,
        }
    }

    #[test]
    fn an_authorization_is_consumed_once() {
        let dir = TempDir::new();
        let mut store = queued(&dir);
        apply(&mut store, vec![consumed(T0 + 1)]).unwrap();
        assert_eq!(
            apply(&mut store, vec![consumed(T0 + 2)]),
            Err("an authorization is consumed once")
        );
        let record = store
            .read(|r| r.authorization(&id("auth-1")))
            .unwrap()
            .unwrap();
        assert_eq!(record.state, AuthorizationState::Consumed);
        assert_eq!(record.authorization.consumed_at, Some(testing::ts(T0 + 1)));
        store.verify_replay().unwrap();
    }

    #[test]
    fn an_expired_authorization_is_never_consumed() {
        let dir = TempDir::new();
        let mut store = queued(&dir);
        assert_eq!(
            apply(&mut store, vec![consumed(T0 + 60_000)]),
            Err("an expired authorization is never consumed")
        );
        apply(&mut store, vec![consumed(T0 + 59_999)]).unwrap();
    }

    #[test]
    fn an_authorization_of_another_run_is_never_consumed() {
        let dir = TempDir::new();
        let mut store = queued(&dir);
        apply(
            &mut store,
            vec![Mutation::RunStarted {
                run_id: 2,
                at_ms: T0,
            }],
        )
        .unwrap();
        assert_eq!(
            apply(&mut store, vec![consumed(T0 + 1)]),
            Err("an authorization of another run is never consumed")
        );
    }

    #[test]
    fn a_voided_authorization_is_never_consumed_or_voided_again() {
        let dir = TempDir::new();
        let mut store = queued(&dir);
        let void = || Mutation::AuthorizationVoided {
            authorization_id: id("auth-1"),
            at_ms: T0,
            reason: VoidReason::FenceChanged,
        };
        apply(&mut store, vec![void()]).unwrap();
        assert_eq!(
            apply(&mut store, vec![consumed(T0 + 1)]),
            Err("a voided authorization is never consumed")
        );
        assert_eq!(
            apply(&mut store, vec![void()]),
            Err("only an unused authorization is voided")
        );
    }

    #[test]
    fn an_authorization_is_issued_in_the_current_run_on_its_agents_grants() {
        let dir = TempDir::new();
        let mut store = testing::world(&dir);
        let issue = |record| Mutation::AuthorizationIssued { record };
        assert_eq!(
            apply(
                &mut store,
                vec![issue(testing::authorization(
                    "auth-1",
                    "request-1",
                    "agent-1",
                    "grant-1",
                    2,
                    T0
                ))]
            ),
            Err("an authorization is issued in the current run")
        );
        assert_eq!(
            apply(
                &mut store,
                vec![issue(testing::authorization(
                    "auth-1",
                    "request-1",
                    "agent-1",
                    "grant-2",
                    1,
                    T0
                ))]
            ),
            Err("an authorization rests on grants of its agent")
        );
        let mut two = testing::authorization("auth-1", "request-1", "agent-1", "grant-1", 1, T0);
        two.grant_ids = vec![id("grant-2"), id("grant-1")];
        assert_eq!(
            apply(&mut store, vec![issue(two)]),
            Err("an authorization names its first grant")
        );
    }

    #[test]
    fn a_dispatch_intent_needs_the_consumption_of_its_authorization() {
        let dir = TempDir::new();
        let mut store = queued(&dir);
        let intent = |number| Mutation::DispatchIntent {
            attempt: testing::attempt("effect-1", "auth-1", number, T0 + 1),
        };
        assert_eq!(
            apply(&mut store, vec![intent(1)]),
            Err("an attempt follows the consumption of its authorization")
        );
        assert_eq!(
            apply(&mut store, vec![consumed(T0 + 1), intent(2)]),
            Err("attempts are numbered from one, in order")
        );
        apply(&mut store, vec![consumed(T0 + 1), intent(1)]).unwrap();
        let effect = store
            .read(|r| r.effect_of(&id("request-1")))
            .unwrap()
            .unwrap();
        assert_eq!(
            (effect.attempts_started, effect.first_attempt_at_ms),
            (1, Some(T0 + 1))
        );
        apply(
            &mut store,
            vec![Mutation::AttemptFinished {
                effect_id: id("effect-1"),
                number: 1,
                finished_at_ms: T0 + 2,
                outcome: fabric_desktop_contracts::owner::AttemptOutcome::Applied,
            }],
        )
        .unwrap();
        let attempts = store.read(|r| r.attempts(&id("effect-1"))).unwrap();
        assert_eq!(attempts.len(), 1);
        assert_eq!(attempts[0].attempt.finished_at, Some(testing::ts(T0 + 2)));
        store.verify_replay().unwrap();
    }

    #[test]
    fn a_counter_never_decreases() {
        let dir = TempDir::new();
        let mut store = testing::world(&dir);
        let raise = |to| Mutation::CounterRaised {
            counter: Counter::RevocationEpoch,
            to,
        };
        apply(&mut store, vec![raise(2)]).unwrap();
        assert_eq!(
            apply(&mut store, vec![raise(1)]),
            Err("a counter never decreases")
        );
        assert_eq!(store.read(|r| r.counters()).unwrap().revocation_epoch, 2);
    }

    #[test]
    fn a_relied_grant_belongs_to_the_requests_agent() {
        let dir = TempDir::new();
        let mut store = testing::world(&dir);
        let relied = |grant: &str| Mutation::ReliedGrantSet {
            request_id: id("request-1"),
            grant_ids: vec![id(grant)],
        };
        assert_eq!(
            apply(&mut store, vec![relied("grant-2")]),
            Err("a relied grant belongs to the request's agent")
        );
        assert_eq!(
            apply(&mut store, vec![relied("grant-unknown")]),
            Err("a relied grant belongs to the request's agent")
        );
        apply(&mut store, vec![relied("grant-1")]).unwrap();
        let record = store
            .read(|r| r.request(&id("request-1")))
            .unwrap()
            .unwrap();
        assert_eq!(record.relied_grant_ids, vec![id("grant-1")]);
    }

    fn reserve(effect: EffectRow) -> Mutation {
        Mutation::EffectReserved { effect }
    }

    #[test]
    fn only_a_create_reserves_its_event_id_and_always() {
        let dir = TempDir::new();
        let mut store = testing::world(&dir);
        let mut create = testing::effect("effect-1", "request-1", CREATE);
        create.provider_event_id = None;
        assert_eq!(
            apply(&mut store, vec![reserve(create)]),
            Err("only a create reserves its event id, always")
        );
        let delete = "fabric_calendar_propose_delete";
        apply(
            &mut store,
            vec![Mutation::RequestInserted {
                row: testing::request("request-3", "agent-1", delete),
                due_at_ms: None,
            }],
        )
        .unwrap();
        let mut with_id = testing::effect("effect-3", "request-3", delete);
        with_id.provider_event_id = Some("fabriccreated0003".to_owned());
        assert_eq!(
            apply(&mut store, vec![reserve(with_id)]),
            Err("only a create reserves its event id, always")
        );
        apply(
            &mut store,
            vec![
                reserve(testing::effect("effect-1", "request-1", CREATE)),
                reserve(testing::effect("effect-3", "request-3", delete)),
            ],
        )
        .unwrap();
    }

    #[test]
    fn a_profile_version_never_touches_the_profiles_revision() {
        let dir = TempDir::new();
        let mut store = testing::world(&dir);
        let profile = |version: u64| -> fabric_desktop_contracts::owner::CustomProfile {
            testing::from(serde_json::json!({
                "profile_id": "custom-0011223344556677",
                "name": "Mine",
                "version": version,
                "rows": fabric_desktop_contracts::owner::ProfileRows::CAREFUL,
            }))
        };
        apply(
            &mut store,
            vec![Mutation::ProfileVersionAdded {
                profile: profile(1),
                at_ms: T0,
            }],
        )
        .unwrap();
        let profiles = store.read(|r| r.profiles()).unwrap();
        assert_eq!(profiles.revision.get(), 0);
        assert_eq!(profiles.items.len(), 3);
        assert_eq!(
            apply(
                &mut store,
                vec![Mutation::ProfileVersionAdded {
                    profile: profile(3),
                    at_ms: T0,
                }]
            ),
            Err("a profile's version rises by one")
        );
        let set = |revision| Mutation::ProfileStateSet {
            active_profile_id: id("custom-0011223344556677"),
            active_version: 1,
            revision,
        };
        assert_eq!(
            apply(&mut store, vec![set(2)]),
            Err("the profiles revision rises by one")
        );
        apply(&mut store, vec![set(1)]).unwrap();
        let (active, version, rows) = store.read(|r| r.active_profile()).unwrap();
        assert_eq!(
            (active.as_str(), version, rows),
            (
                "custom-0011223344556677",
                1,
                fabric_desktop_contracts::owner::ProfileRows::CAREFUL
            )
        );
        assert_eq!(
            apply(
                &mut store,
                vec![Mutation::ProfileStateSet {
                    active_profile_id: id("custom-unknown"),
                    active_version: 1,
                    revision: 2,
                }]
            ),
            Err("the active profile exists")
        );
    }

    #[test]
    fn a_session_starts_live_in_the_current_run_and_ends_once() {
        let dir = TempDir::new();
        let mut store = testing::world(&dir);
        let session = |run_id| SessionRow {
            session_id: id("session-1"),
            agent_id: id("agent-1"),
            run_id,
            started_at_ms: T0,
            ended: None,
        };
        assert_eq!(
            apply(
                &mut store,
                vec![Mutation::SessionStarted {
                    session: session(2)
                }]
            ),
            Err("a session starts in the current run")
        );
        apply(
            &mut store,
            vec![Mutation::SessionStarted {
                session: session(1),
            }],
        )
        .unwrap();
        let end = || Mutation::SessionEnded {
            session_id: id("session-1"),
            at_ms: T0,
            reason: SessionEndReason::Replaced,
        };
        apply(&mut store, vec![end()]).unwrap();
        assert_eq!(
            apply(&mut store, vec![end()]),
            Err("only a live session ends")
        );
        let row = store
            .read(|r| r.session(&id("session-1")))
            .unwrap()
            .unwrap();
        assert_eq!(row.ended, Some((T0, SessionEndReason::Replaced)));
    }

    #[test]
    fn re_proof_counts_never_decrease() {
        let dir = TempDir::new();
        let mut store = testing::world(&dir);
        let schedule = |classification, fresh_permit| Mutation::RequestScheduled {
            request_id: id("request-1"),
            due_at_ms: None,
            reproofs: crate::rows::Reproofs {
                classification,
                fresh_permit,
            },
        };
        apply(&mut store, vec![schedule(1, 2)]).unwrap();
        assert_eq!(
            apply(&mut store, vec![schedule(0, 2)]),
            Err("re-proof counts never decrease")
        );
    }

    #[test]
    fn a_pending_request_carries_its_card_and_its_decision() {
        let dir = TempDir::new();
        let mut store = testing::world(&dir);
        let pending: LifecycleStatus = testing::from(serde_json::json!({
            "state": "pending_approval",
            "approval_expires_at": testing::ts(T0 + 86_400_000),
        }));
        let card: fabric_desktop_contracts::owner::ApprovalCard =
            testing::from(serde_json::json!({
                "scope": {
                    "calendars": [{ "account_id": "account-1", "calendar_id": "calendar-1" }],
                    "window": null, "fields": null, "recipients": null, "send_updates": null,
                },
                "proposed_duration": { "kind": "one_request" },
                "insufficiency": { "rule": "needs_approval", "explanation": "Ask.",
                                   "proof_id": "proof-1" },
                "expires_at": testing::ts(T0 + 86_400_000),
            }));
        let approval: fabric_desktop_contracts::owner::Approval =
            testing::from(serde_json::json!({
                "decision": "deny",
                "command_id": "command-1",
                "decided_at": testing::ts(T0),
                "approval_digest": testing::digest('f'),
                "scope": null, "duration": null, "effective_request_digest": null,
                "grant_id": null,
            }));
        let record = |approval| Mutation::ApprovalRecorded {
            request_id: id("request-1"),
            approval,
        };
        assert_eq!(
            apply(&mut store, vec![record(approval.clone())]),
            Err("only a pending request is decided")
        );
        let to_pending = Mutation::RequestMoved {
            request_id: id("request-1"),
            revision: 4,
            status: pending,
            at_ms: T0,
            due_at_ms: Some(T0 + 86_400_000),
            undecided_cause: None,
        };
        let path = || {
            vec![
                moved("request-1", 2, "validating"),
                moved("request-1", 3, "proving_classification"),
            ]
        };
        let mut without_card = path();
        without_card.push(to_pending.clone());
        assert!(matches!(
            testing::apply_all(&mut store, T0, without_card),
            Err(TxError::Fault(crate::StoreFault::Sql { .. }))
        ));
        let mut with_card = path();
        with_card.push(Mutation::CardOpened {
            request_id: id("request-1"),
            card,
            opened_at_ms: T0,
        });
        with_card.push(to_pending);
        with_card.push(record(approval.clone()));
        apply(&mut store, with_card).unwrap();
        let request = store
            .read(|r| r.request(&id("request-1")))
            .unwrap()
            .unwrap();
        assert_eq!(request.card_expires_at_ms, Some(T0 + 86_400_000));
        assert_eq!(request.approval, Some(approval));
        assert_eq!(store.read(|r| r.count_cards(&id("agent-1"))).unwrap(), 1);
        store.verify_replay().unwrap();
    }

    #[test]
    fn a_grant_belongs_to_its_origin_requests_agent() {
        let dir = TempDir::new();
        let mut store = testing::world(&dir);
        assert_eq!(
            apply(
                &mut store,
                vec![Mutation::GrantInserted {
                    grant: testing::grant("grant-3", "agent-1", "request-2"),
                }]
            ),
            Err("a grant belongs to its origin request's agent")
        );
    }

    #[test]
    fn recipient_sets_rise_by_one_and_stay_deleted() {
        let dir = TempDir::new();
        let mut store = testing::world(&dir);
        let set = |revision: u64| -> fabric_desktop_contracts::owner::RecipientSet {
            testing::from(serde_json::json!({
                "recipient_set_id": "set-1",
                "revision": revision,
                "source": { "kind": "local" },
                "definition": { "name": "Team", "members": [], "domains": ["example.com"] },
            }))
        };
        let add = |revision| Mutation::RecipientSetVersionAdded {
            set: set(revision),
            at_ms: T0,
        };
        apply(&mut store, vec![add(1), add(2)]).unwrap();
        assert_eq!(
            apply(&mut store, vec![add(4)]),
            Err("a recipient set's revision rises by one")
        );
        assert_eq!(
            store.read(|r| r.recipient_set(&id("set-1"))).unwrap(),
            Some(set(2))
        );
        let delete = || Mutation::RecipientSetDeleted {
            recipient_set_id: id("set-1"),
            at_ms: T0,
        };
        apply(&mut store, vec![delete()]).unwrap();
        assert_eq!(
            apply(&mut store, vec![delete()]),
            Err("only a current recipient set is deleted")
        );
        assert_eq!(
            apply(&mut store, vec![add(3)]),
            Err("a deleted recipient set stays deleted")
        );
        assert!(store.read(|r| r.recipient_sets()).unwrap().is_empty());
        store.verify_replay().unwrap();
    }

    // The two tests below write the projection beside `apply`, as a tampering process would;
    // that SQL lives here because only this module may hold projection writes (s16).

    #[test]
    fn replay_rebuilds_the_projection_and_finds_a_change_made_beside_it() {
        let dir = TempDir::new();
        let store = testing::world(&dir);
        store.verify_replay().unwrap();
        store
            .connection()
            .execute(
                "UPDATE counters SET value = 5 WHERE name = 'pause_epoch'",
                [],
            )
            .unwrap();
        assert_eq!(
            store.verify_replay(),
            Err(crate::StoreFault::Chain {
                seq: 2,
                what: "replay differs in counters"
            })
        );
    }

    #[test]
    fn a_corrupted_column_is_a_decode_fault_without_its_content() {
        let dir = TempDir::new();
        let store = testing::world(&dir);
        let marker = "FABRIC-W04-PLAINTEXT-MARKER-unit";
        store
            .connection()
            .execute(
                "UPDATE agents SET record = ?1 WHERE agent_id = 'agent-1'",
                [format!("{{\"agent_id\":\"{marker}\"")],
            )
            .unwrap();
        let fault = store.read(|r| r.agent(&id("agent-1"))).unwrap_err();
        assert_eq!(
            fault,
            crate::StoreFault::Decode {
                table: "agents",
                column: "record"
            }
        );
        assert!(!fault.to_string().contains(marker));
        assert!(!format!("{fault:?}").contains(marker));
    }
}
