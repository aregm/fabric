# Design doctrine

## Contents

1. Working vocabulary
2. Requirements grammar
3. Conceptual design
4. Decomposition and abstraction
5. Interface and composition rules
6. Case-study lenses

## Working vocabulary

- **Purpose:** stakeholder intention. Follow “why?” until the answer reaches a current commitment.
- **Operand:** what the system transforms, moves, remembers, coordinates, or regulates.
- **Function:** behavior applied to an operand without prematurely naming form.
- **Form:** concrete structure and mechanism selected to realize functions.
- **Concept:** coherent organizing idea connecting purpose and functions to form.
- **Boundary:** chosen system of interest where responsibility meets context.
- **Formal relationship:** how parts are structurally connected.
- **Functional relationship:** how parts cooperate to produce system behavior.
- **Invariant:** a property that remains true across permitted histories.
- **Requisite variety:** a regulator must distinguish enough relevant states to respond to the environment it claims to control.
- **Conceptual integrity:** governing ideas appear consistently in vocabulary, boundaries, interfaces, data, and behavior.

## Requirements grammar

A requirement is an obligation about the world, traceable to a stakeholder reason and stated so satisfaction is observable. A specification constrains behavior at the machine boundary. Domain assumptions connect the two and deserve explicit ownership.

Use: `stakeholder intent → world requirement → domain assumptions → machine specification → evidence`.

Prioritize because a system where everything is mandatory cannot be optimized coherently. Distinguish inherent complexity imposed by accepted behavior from contributed complexity created by the solution. The only reliable way to lower inherent complexity is to require less.

Elicit from practice, exceptions, artifacts, observed work, failures, and disagreement—not only interviews. Terms that groups use confidently but differently are architectural risks.

## Conceptual design

Derive: `purpose → operand and attributes → functions → concept → form`.

A metaphor imports unstated requirements. For each metaphor, list units and identity, interaction grammar, authority, ordering/time, failure/recovery, composition rule, and awkward cases. Mixed metaphors are valid only with an explicit seam and translation semantics.

## Decomposition and abstraction

Functional decomposition mirrors present behavior but can scatter future changes. Domain decomposition aligns language and ownership but can duplicate cross-domain mechanisms. Object decomposition helps encapsulation but can fossilize taxonomies. Use volatility-based decomposition for long-lived systems: isolate policies and mechanisms that change for different reasons.

A decomposition is a bet about future change. Test it through composition and change scenarios.

Prefer deep modules: a small interface hides substantial complexity and transfers responsibility inward. Reject shallow modules that rename mechanics or require every caller to understand the implementation.

Give each consequential decision one authoritative home. Separate stable facts from mutable intentions, logical meaning from physical representation, and desired state from observed state.

## Interface and composition rules

Cover semantic contract, invariants, ownership, timing, failure, concurrency, capacity, evolution, and trust. Minimum distributed outcomes are success, retryable failure, terminal failure, and ambiguous outcome.

Exactly-once delivery is not generally available. Build exactly-once effect above transport with stable operation identity, deduplication, durable state, and idempotent transitions.

Composition residue—permanent adapters, duplicated decisions, translation layers, shared databases without authority rules, or context known only by people—suggests a misplaced seam.

## Case-study lenses

- **Unix:** small composable grammar and information hiding; universal text/file interfaces trade semantic richness for reach.
- **Git:** immutable content-addressed facts separated from mutable names and candidate state; one ontology supports many operations.
- **Build systems:** explicit causality enables incrementality, caching, explanation, and reproducibility; correctness precedes minimal rebuilding.
- **Relational databases:** stable logical meaning separated from volatile physical form; transactions specify acceptable histories.
- **PageRank:** shift the operand to expose a tractable concept; fixed points require explicit domain assumptions and self-falsification.
- **LLM-assisted development:** generated code is a proposal; independent constraints reject known violations; accountable people own changes to meaning.
