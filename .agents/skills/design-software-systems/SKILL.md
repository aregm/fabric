---
name: design-software-systems
description: Guide rigorous software architecture from ambiguous intent through requirements, conceptual design, decomposition, interfaces, implementation constraints, and validation. Use when Codex must design or redesign a software system, evaluate an architecture, define module or service boundaries, write an architecture specification or ADRs, analyze quality attributes and tradeoffs, plan migrations, review an architecture-affecting change, or turn architectural decisions into executable guardrails. Especially appropriate when requirements are incomplete, the system must evolve, multiple teams or trust boundaries are involved, or an LLM will implement against the design.
---

# Design Software Systems

Design for coherence under change. Treat architecture as a derivation from intent, not a catalog of fashionable components.

## Operating principles

- Separate inherent complexity demanded by behavior from complexity contributed by construction.
- Treat decomposition and composition as one loop: boundaries are valid only when the parts compose without unexplained residue.
- Make assumptions, rejected alternatives, and uncertainty visible. Never invent missing intent silently.
- Prefer the simplest form that satisfies current obligations and preserves the named change seams.
- Treat an interface as a relationship with semantics, time, failure, ownership, and evolution—not only a signature.
- Give each invariant one authoritative owner.
- Turn consequential recurring decisions into executable constraints where practical.
- Distinguish verification (built according to specification) from validation (specified the right system).

Read [references/design-doctrine.md](references/design-doctrine.md) when deriving a new architecture or resolving conceptual ambiguity. Read [references/review-and-validation.md](references/review-and-validation.md) for architecture reviews, quality scenarios, ADRs, migrations, or LLM-generated changes. Use [assets/architecture-dossier.md](assets/architecture-dossier.md) as the default deliverable skeleton when the user requests a design document.

## Workflow

### 1. Establish the system of interest

State purpose, stakeholders, desired outcomes, explicit non-goals, operand, boundary, environment, external actors, trust boundaries, current state, migration constraints, decision horizon, and expected lifetime.

Ask only questions whose answers would materially change the concept or boundary. If answers are unavailable, record bounded assumptions and their consequences.

### 2. Convert intent into testable obligations

For each consequential requirement, capture:

`stakeholder reason → world condition → machine-boundary specification → verification evidence`

Separate functional obligations, quality-attribute scenarios, constraints, domain assumptions, priorities, and non-goals. Reject vague qualities such as “scalable” or “secure.” Express a quality scenario with source, stimulus, environment, affected artifact, response, and measurable response criterion.

### 3. Derive the conceptual design

Write the derivation explicitly:

`purpose → operand → required attributes → functions → governing concept → candidate form`

Name the governing metaphor or grammar when one exists: pipeline, ledger, marketplace, control loop, language, graph, postal system, and so on. Enumerate what the metaphor silently implies. Mark each implication accepted, rejected, or load-bearing.

Generate at least two conceptually distinct candidates for consequential designs. Do not compare forms that share the same hidden concept and call them alternatives.

### 4. Choose boundaries by volatility and authority

Identify what changes at different rates, for different reasons, under different owners or policies, and across trust, failure, deployment, or regulatory boundaries. Place seams to localize those changes. Prefer deep modules: narrow, stable interfaces hiding substantial policy or mechanism. Avoid distributing one decision across several components.

For every proposed module or service, state responsibility, owned decisions, state and invariants, public contract, dependencies and forbidden dependencies, volatility isolated, failure containment, and why independent deployment is required. Do not infer microservices from team count or anticipated scale alone.

### 5. Design composition and interface grammar

For each important relationship, specify:

- semantic contract, preconditions, postconditions, and invariants;
- state ownership and mutation authority;
- synchronous or asynchronous interaction;
- ordering, delivery, and consistency semantics;
- retryable, terminal, and ambiguous outcomes;
- timeouts, retry budgets, backoff, jitter, and cancellation;
- idempotency and deduplication;
- backpressure or load shedding;
- authentication, authorization, privacy, and audit;
- versioning and wire/source/semantic/behavioral compatibility;
- observability and operational ownership.

Check mechanism pairs. Retries require idempotency; timeouts require budgets; asynchrony requires correlation and compensation; invariants require ownership; version numbers require a compatibility policy.

Treat adapters, duplicated policy, translation layers, and shared unwritten context as composition residue. Revisit the boundary before normalizing the residue.

### 6. Select concrete form last

Choose data models, storage engines, processes, deployment topology, protocols, frameworks, and vendors only after preceding decisions constrain them.

For each major choice, record the requirement and quality served, accepted concept, rejected alternatives, cost and operational burden, assumptions and evidence, and reconsideration trigger.

Separate logical, process, deployment, data, security, and operational views. Do not use one box-and-arrow diagram to answer every question.

### 7. Validate the architectural bets

Test the design with normal and adversarial usage; change across named volatility seams; failure, overload, recovery, and partial-success histories; security and abuse cases; data evolution and migration rehearsals; prototypes with a predeclared negative result; counterfactual designs; and cost/operability exercises.

Identify sensitivity points, tradeoff points, single points of authority/failure, irreversible decisions, and claims with weak evidence. If no possible validation result would change the design, the activity is a demonstration rather than validation.

### 8. Make decisions durable and executable

Produce artifacts that carry decisions: context and derivation chain, requirement/assumption traceability, module responsibilities, interface contracts, data ownership, invariants, quality scenarios, threat model, ADRs, migration/rollback, verification matrix, and residual risks.

Encode stable consequential rules in schemas, types, dependency constraints, tests, model checks, policy checks, or production monitors. Give every guardrail an owner, scope, enforcement point, and reconsideration condition. Keep the examiner independent from generated implementation.

## Review mode

When reviewing an existing design, reconstruct before redesigning:

1. What purpose and operand does the design imply?
2. Which concept governs it?
3. Which decisions and invariants live where?
4. Which volatility assumptions determined the seams?
5. What composition residue and accidental coupling exist?
6. Which qualities are sensitive to which decisions?
7. What evidence supports the claims?

Report findings by consequence and cite the relevant artifact. Distinguish concept errors, boundary errors, interface-contract gaps, unsupported quality claims, migration risks, and local implementation defects.

## Output standard

Lead with the architectural decision and its derivation. Use diagrams only when they clarify relationships, ownership, runtime sequence, state transitions, or deployment. Label assumptions and open decisions. Give acceptance criteria and evidence for important claims. Avoid presenting estimates, vendor promises, or pattern names as proof.
