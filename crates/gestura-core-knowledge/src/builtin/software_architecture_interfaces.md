# Software: Architecture & Interfaces

Use this module when the task is defining boundaries, APIs, schemas, contracts, or structural choices in a production software system.

## Focus

- Architecture should reflect ownership, constraints, and likely change surfaces.
- Interfaces should make invariants, data contracts, and failure semantics explicit.
- Good boundaries reduce accidental coupling and operational confusion.

## High-Value Defaults

- Clarify requirements, constraints, and non-goals before picking patterns.
- Prefer simple interfaces and explicit ownership.
- Keep data contracts and failure modes visible.
- Design for operability, rollback, and debuggability.
- Call out coupling that will make future change expensive.

## Review Lens

- Who owns each boundary and how will it evolve?
- What invariants must hold across the interface?
- Where can partial failure occur and how is it surfaced?
- Which future changes would force this structure to be revisited?

## Common Failure Modes

- Over-abstracting before real seams are understood.
- Hiding coupling in shared tables, utility layers, or implicit side effects.
- Designing APIs without migration, versioning, or error semantics.
- Splitting components in ways the team cannot operate reliably.

## Good Outputs

- Boundary maps with ownership and contracts.
- API or schema guidance with invariants and versioning notes.
- Tradeoff summaries on latency, consistency, and coupling.
- Recommendations for interface tests and observability hooks.

## Retrieval Hints

architecture, system boundary, API contract, schema design, ownership, integration surface, coupling, invariants, versioning, failure semantics.

## Authoritative Sources

- Google SRE Book
- AWS Well-Architected
- Martin Fowler