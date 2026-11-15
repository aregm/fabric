# Architecture dossier: <system>

## Executive decision

State the selected concept and why it best satisfies the highest-priority obligations.

## 1. Context and system of interest

- Purpose:
- Stakeholders:
- Operand:
- Boundary and environment:
- Trust boundaries:
- Current state:
- Non-goals:
- Decision horizon:

## 2. Requirements and assumptions

| ID | Stakeholder reason | World condition | Machine specification | Priority | Evidence |
|---|---|---|---|---|---|

### Quality-attribute scenarios

| ID | Source | Stimulus | Environment | Artifact | Response | Measure |
|---|---|---|---|---|---|---|

### Assumptions

| ID | Assumption | Owner | Consequence if false | Validation |
|---|---|---|---|---|

## 3. Conceptual derivation

`purpose → operand/attributes → functions → concept → form`

### Governing metaphor and entailments

| Entailment | Accepted/rejected/load-bearing | Architectural consequence |
|---|---|---|

### Alternatives

| Candidate concept | Strengths | Failure mode | Reason accepted/rejected |
|---|---|---|---|

## 4. Decomposition

| Module | Owned decision/state | Invariants | Public contract | Volatility isolated | Failure boundary |
|---|---|---|---|---|---|

### Forbidden dependencies

List edges that would collapse an intended seam.

## 5. Interface and composition contracts

For each significant relationship specify semantics, ownership, timing, failures, idempotency, capacity, security, compatibility, and observability.

## 6. Data and authority

- Canonical model:
- Authoritative owner per decision:
- Consistency and transaction boundaries:
- Retention/deletion:
- Privacy and key boundaries:

## 7. Runtime and deployment views

Include only diagrams that answer a named question.

## 8. Failure, recovery, and operations

- Failure histories:
- Degraded modes:
- Recovery/reconciliation:
- SLOs and alerts:
- Capacity and cost model:

## 9. Security and abuse cases

List assets, actors, entry points, privileges, threats, controls, residual risk, and verification.

## 10. Evolution and migration

- Change scenarios:
- Compatibility window:
- Data migration:
- Rollout and rollback:
- Exit strategy:

## 11. Decisions

| ADR | Decision | Quality served | Owner | Reconsideration trigger |
|---|---|---|---|---|

## 12. Validation and evidence

| Claim | Method | Configuration/boundary | Result | Residual uncertainty |
|---|---|---|---|---|

## 13. Open decisions and risks

| Item | Consequence | Owner | Needed by |
|---|---|---|---|
