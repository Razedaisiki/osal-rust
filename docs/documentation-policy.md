# Documentation Policy

## Purpose

This document defines the authority of each document in the OSAL
repository, when each must be updated, and the status terminology
used throughout.

## Authority model

Two distinct kinds of authority exist in this repository:

- **Compilation-surface authority**: Rust public APIs and Cargo
  manifests are authoritative for the currently available compilation
  surface — signatures, types, feature flags, and dependency edges.
- **Semantic authority**: `docs/behavior-contract.md` is authoritative
  for intended observable backend semantics — what every backend
  **MUST** do at runtime.

When an implementation disagrees with the behavior contract, the
disagreement is a **conformance defect**; the implementation does not
silently redefine the contract.

### Resolution order

When two documents make conflicting claims about backend semantics,
resolve using this order:

1. **`docs/behavior-contract.md`** — normative backend conformance
   requirements. Describes what backends **MUST** do.

2. **`docs/adr/*.md`** — why specific design decisions were made.
   Accepted ADRs record the rationale at the time of decision.

3. **`docs/architecture.md`** — stable layer boundaries, dependency
   directions, and crate responsibility descriptions.

4. **`docs/*-foundation-slice.md`** — per-capability implementation
   status, component breakdown, and deferred items for a specific
   feature slice.

5. **`README.md`** — summary snapshot for new contributors. Must not
   introduce new semantic claims not present in higher-priority
   documents.

6. **`CHANGELOG.md`** — record of what changed when, grouped by
   development phase.

Rust public APIs and Cargo manifests are the authority on what
compiles, not on what the intended runtime semantics are.

## Document responsibilities

| Document | Answers | Audience |
|----------|---------|----------|
| Code / rustdoc | "What does it do right now?" | Compiler, IDE, all developers |
| Behavior contract | "What must every backend do?" | Backend implementors, test authors |
| ADRs | "Why was this decision made?" | Maintainers, future contributors |
| Architecture | "How do the pieces fit together?" | New contributors, reviewers |
| Foundation slices | "What is done and what is deferred for X?" | Contributors to feature X |
| README | "What is this project and where is it?" | First-time visitors |
| CHANGELOG | "What changed between milestones?" | Upgrading users |
| Documentation policy | "How do I keep docs consistent?" | All contributors |

## When a document must be updated

### API signature change

- Code and rustdoc: always
- Behavior contract: if the change affects observable semantics
- CHANGELOG: always (under the current phase)

### Behavior change (same API, different outcome)

- Code and tests: always
- Behavior contract: always (normative behavior changed)
- ADR: if architecturally significant
- CHANGELOG: always

### Implementation-only change (refactor, perf, fix without API change)

- Code and tests: always
- CHANGELOG: when externally relevant (bug fix, performance)

### New architecture decision

- New ADR with sequential number
- ADR index in README updated
- Architecture document: when layer boundaries or dependencies change

### New milestone

- CHANGELOG: new phase section
- README: status section updated

### When a capability moves from Deferred to Implemented

- Behavior contract: remove Deferred markers, add normative
  requirements
- Architecture document: update crate maturity labels
- Foundation slice: update status
- README: update capability matrix

## Status terminology

Capability status and crate maturity are separate vocabularies.
Capability status describes behavioral completeness of a feature
(Queue, Mutex, Runtime Lifecycle, etc.). Crate maturity describes
whether a workspace crate is actively maintained, stabilizing, a
placeholder, or not yet created.

### Capability status

Use these terms in the README capability matrix and foundation
slice documents:

| Term | Meaning |
|------|---------|
| `Validated` | API, implementation, and contract tests complete |
| `Implemented` | Implemented; contract or edge-case verification ongoing |
| `Foundation` | Foundation semantics complete; advanced features deferred |
| `Planned` | Design exists or sketched; implementation not started |
| `Deferred` | Explicitly deferred to a future phase with recorded rationale |
| `N/A` | Not applicable to this layer or backend |

### Crate maturity

Use these terms in `architecture.md` and when discussing workspace
crate lifecycle:

| Term | Meaning |
|------|---------|
| `Active` | Crate is actively developed and maintained |
| `Stabilizing` | Core implementation complete; API surface settling |
| `Skeleton` | Crate exists as workspace placeholder only (no runtime logic) |
| `Planned` | Design exists; crate not yet created |

## ADR rules

- Accepted ADRs **MUST NOT** be silently rewritten. When a design
  changes, create a new ADR and mark the old one as **Superseded**
  with a link to the replacement.
- ADR numbers are sequential and never reused.
- Each ADR **MUST** record its status (`Proposed`, `Accepted`,
  `Superseded`, or `Deprecated`) and the date of status change.
- The ADR index in `README.md` **MUST** list every ADR in numeric
  order.

## Marking Deferred or Future work

- Use **Deferred** for items with a recorded decision to postpone
  (usually linked to an ADR or foundation slice).
- Use **Future** or **Planned** for items on the roadmap without a
  formal deferral decision.
- Behavior contract sections marked **Deferred** are not part of
  current backend conformance requirements.
- Architecture document must distinguish between current
  implementation and target extension using separate diagrams or
  clearly labelled sections.

## Review checklist

Before marking a document change complete, verify:

- [ ] No new semantic claims introduced in README that aren't in
  behavior-contract or architecture
- [ ] Feature flags and allocation model consistent across
  architecture.md and behavior-contract.md §2
- [ ] Unimplemented capabilities (EventFlags, ISR traits, BSP) not
  described as current
- [ ] ADR index in README includes all ADRs
- [ ] CHANGELOG phase numbering matches README status
- [ ] Status terminology matches the table above
