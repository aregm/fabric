# Architecture review and validation

## Contents

1. Quality scenarios
2. Change and failure review
3. Decision records
4. Architecture fitness functions
5. LLM-assisted change review
6. Review questions

## Quality scenarios

| Element | Question |
|---|---|
| Source | Who or what creates the stimulus? |
| Stimulus | What event occurs? |
| Environment | Under what load, failure, deployment, or operational condition? |
| Artifact | Which component, relationship, data, or operation is affected? |
| Response | What must the system do? |
| Measure | What observable threshold decides success? |

Build a utility tree by quality, scenario, business importance, and architectural risk. Identify sensitivity points where one decision strongly affects a quality and tradeoff points where one decision affects several qualities in conflict.

## Change and failure review

Apply scenarios: replace a provider/protocol; add a tenant, jurisdiction, locale, device, or trust domain; change identity or retention policy; evolve schema with mixed versions; lose a dependency, key, region, or replica; receive duplicate/delayed/reordered/conflicting work; overload queues; crash around durable decisions; migrate or undo a choice; transfer ownership.

For each, record touched modules/contracts, decision owner, data migration, compatibility window, operational steps, rollback, and evidence.

Use prototypes only with a falsification condition declared first. Test the deployed topology rather than a simplified surrogate.

## Decision records

An ADR contains context and live quality served, decision and governing concept, alternatives including status quo, consequences and operational burden, assumptions and evidence, owner, executable manifestations, and conditions that reopen/retire it.

Keep ADRs short enough to exist. A rationale terminating in history rather than current need cannot be revalidated.

## Architecture fitness functions

Choose enforcement strength by consequence, recurrence, observability, and stability.

| Surface | Mechanism | Claim boundary |
|---|---|---|
| Boundary values | Schema/validation | Representable and accepted data |
| Constructible code | Types/capabilities | Programs expressible through the API |
| Static relationships | Dependency/import rules | Visible graph edges |
| Executions | Unit/integration/property/fault tests | Sampled or generated histories |
| State machines | Model checking | Declared model and explored bounds |
| Mathematical claims | Proof | Formal specification and trusted base |
| Production behavior | Monitors/SLOs/audit | Observed telemetry and traffic |

Every guardrail needs a named decision, owner, scope, enforcement point, actionable failure, and reconsideration condition. A growing exception list indicates the constraint or boundary has expired.

## LLM-assisted change review

Review the decision delta:

1. Requested behavior and non-goals.
2. Modules and public contracts changed.
3. Architectural decisions touched or reopened.
4. Dependencies and authority moved.
5. New assumptions, privileges, failure modes, and external effects.
6. Specifications/checkers changed separately from implementation.
7. Evidence with tool versions, configuration, and limits.
8. Claims still dependent on judgment.

Do not allow a candidate implementation to weaken its examiner silently. Separate generation, specification, validation, and acceptance authority where risk warrants it.

## Review questions

- Is the purpose current, and is the boundary explicit?
- What complexity is inherent, and what did construction contribute?
- What concept governs the architecture, and where is it contradicted?
- Which assumptions connect world requirements to machine behavior?
- Does each invariant have one owner able to maintain it?
- Are seams aligned with independent reasons to change?
- Are modules deep, or do callers carry their complexity?
- Do interactions cover ambiguity, overload, retries, and evolution?
- Which mechanisms are unsafe without complements?
- Where does composition leave residue?
- Which qualities depend sensitively on which decisions?
- What is the migration path from the real current state?
- What result would falsify the recommendation?
- What evidence exists, and where does each claim end?
- Which constraints can execute rather than rely on memory?
