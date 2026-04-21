# DD-060: AI-Native Developer Experience for ntnt

## Status: draft

## Problem
The developer bottleneck is changing. As AI coding becomes a primary way software gets produced, the dominant pain points are shifting away from raw typing speed and toward verification, trust, refactor safety, operational coherence, and low-friction supervision.

Python succeeded in the human-first era because it optimized for readability, approachability, batteries-included utility, strong interop, and broad adoption by non-specialists. Emerging discussion around AI-assisted coding suggests a new opening: developers increasingly want systems that preserve Python's low-friction usefulness while offering much stronger correctness guarantees and less runtime babysitting.

ntnt is already pointed in this direction through intent-driven development, an integrated runtime, a constrained app model, and a batteries-included philosophy. However, the current roadmap does not yet explicitly organize around the AI-native developer experience as a first-class product strategy.

If we do not make this explicit, ntnt risks underselling its real advantage and underinvesting in the exact verification, diagnostics, paved-road workflows, and refactor-safety capabilities that AI-era developers and domain experts will increasingly need.

## Solution
Create a roadmap layer and product narrative for ntnt centered on **AI-native developer experience**.

This does **not** mean optimizing ntnt for autonomous code generation at the expense of humans. It means optimizing ntnt to be:
- easy for humans to supervise
- easy for AI to generate within safely
- easy to verify before shipping
- easy to operate end-to-end as an app platform

The strategic goal is for ntnt to become the best environment for building small-to-medium web software with humans and agents working together.

Core framing:
- Python won because it was the most approachable control layer for getting real work done.
- ntnt should aim to be the most trustworthy control layer for AI-mediated app building.

## Design

### Strategic Thesis
The AI era changes what developers need from a language/runtime.

Historically, the winning package was:
- low ceremony
- readable code
- batteries included
- easy interoperability
- easy onboarding for non-specialists

Now the winning package is becoming:
- low ceremony
- readable and reviewable code
- earlier correctness checks
- stronger intent preservation
- refactor safety
- better diagnostics for iterative repair loops
- integrated operational path from app code to deployable system

ntnt should deliberately combine both sets rather than choosing only one side.

### Core Product Principles

#### 1. Easy to supervise
The language should not merely be easy to write. It should be easy to:
- inspect
- review
- explain
- test against intent
- debug when AI takes the wrong turn

This implies:
- syntax remains cognitively compact
- defaults remain obvious
- hidden behavior is minimized
- diagnostics are concrete and actionable

#### 2. More correctness before runtime
As AI increases code volume, runtime bug discovery becomes less acceptable. The human cannot remain the primary static analyzer.

This implies:
- stronger linting and type-adjacent verification where high signal exists
- route/data/auth/job checks that catch real app failures early
- more ways to prove app behavior before deploy

#### 3. Constrained semantic surface
AI performs better in environments with fewer ambiguous or overlapping ways to do the same thing.

This implies:
- preserve the batteries-included posture
- resist framework sprawl
- prefer one good paved road over many competing idioms
- keep stdlib and app patterns coherent

#### 4. Whole-product experience beats language cleverness
Python won on total usefulness, not language purity.

ntnt should therefore optimize not just syntax but the full workflow:
- author
- lint
- intent-check
- run
- debug
- document
- deploy
- operate

#### 5. Domain experts matter
A significant share of future software builders will be founders, operators, analysts, scientists, and engineers using AI as a force multiplier rather than traditional full-time programmers.

ntnt should be usable by:
- developers with AI
- domain experts with AI
- teams that want software without assembling a giant stack of glue

### Opportunity Areas

#### A. Verification as a product pillar
IDD becomes more important in an agentic world, not less.

ntnt already has a differentiated bet here. We should lean into it and treat verification as a core product surface, not just a language-side extra.

Target outcomes:
- developers can state intended behavior clearly
- AI can generate against those expectations
- ntnt can validate conformance before shipping

#### B. Refactor safety
One of the biggest AI pain points is broad but shallow edits that look plausible and fail later.

ntnt should become unusually strong at safe structural change.

Target outcomes:
- route/linkage drift gets caught early
- symbol and contract changes are easier to validate
- app changes can be made broadly without exploding trust

#### C. Diagnostics for repair loops
In AI-assisted workflows, error quality directly affects productivity.

Target outcomes:
- better compiler/lint diagnostics
- more actionable remediation hints
- less ambiguous failure output
- more machine-legible diagnostic structure where feasible

#### D. Paved-road app patterns
The batteries-included model is more important, not less.

Target outcomes:
- fewer dependency decisions
- fewer operational cliffs
- clearer built-in patterns for forms, auth, jobs, APIs, data access, and deployment

#### E. Positioning
ntnt should not frame itself as a generic Python replacement or a purely elegant new language.

Stronger framing:
- the best language/runtime for building apps safely with AI
- less babysitting per generated line
- more confidence from idea to deploy

### What ntnt Already Has That Supports This Direction
- intent-driven development
- integrated runtime and web app model
- constrained and batteries-included philosophy
- progressive type and lint posture
- docs generation and drift resistance
- built-in jobs/concurrency direction
- fewer framework layers and less ecosystem chaos than mainstream stacks

### What Is Missing or Underdeveloped
- stronger end-to-end verification around real app behavior
- explicit refactor-safety tooling and checks
- more diagnostic depth for AI repair loops
- roadmap-level articulation of AI-native DX as a core theme
- more polished paved-road stories for common app concerns
- clearer external messaging around why ntnt is well-suited to the AI era

## Implementation Checklist

### Phase 1: Strategy and framing
- [ ] Refine this proposal based on review
- [ ] Decide whether AI-native developer experience becomes an explicit roadmap theme
- [ ] Define the one-sentence positioning statement for ntnt in the AI era
- [ ] Audit current docs and landing-page messaging for alignment with this positioning

### Phase 2: Verification roadmap
- [ ] Inventory current ntnt verification surfaces (lint, intent, docs, runtime checks)
- [ ] Identify the highest-value missing checks for real app workflows
- [ ] Propose a concrete expansion plan for intent and contract validation
- [ ] Define success criteria for verification improvements in terms of prevented bug classes

### Phase 3: Refactor safety and diagnostics
- [ ] Identify the most common AI-era refactor failure modes in ntnt apps and core work
- [ ] Propose route/data/auth/job linkage checks that can catch those failures early
- [ ] Design a diagnostics improvement plan focused on actionable error output
- [ ] Evaluate whether machine-legible diagnostic formats should be added or improved

### Phase 4: Paved-road app experience
- [ ] Audit the current built-in story for forms, auth, jobs, APIs, and deployment
- [ ] Identify the biggest gaps between ntnt's current happy path and a trustworthy AI-assisted workflow
- [ ] Propose improvements that reduce dependency sprawl and operational glue
- [ ] Prioritize the paved-road improvements that most directly reduce human babysitting

### Phase 5: External proof and adoption
- [ ] Gather concrete examples where ntnt already outperforms looser stacks for AI-assisted work
- [ ] Turn those into docs, examples, or case studies
- [ ] Identify the narrowest high-leverage wedge market for this story
- [ ] Define how to measure whether this positioning resonates with builders

## Risks
| Risk | Likelihood | Mitigation |
|------|-----------|------------|
| The proposal becomes vague strategy language with no product consequences | Medium | Tie every claimed opportunity to specific checks, tooling, or paved-road improvements |
| ntnt drifts toward overengineering in pursuit of verification | Medium | Prioritize checks that prevent common real-world failures, not theoretical perfection |
| Messaging overpromises what ntnt can currently deliver | High | Separate current strengths from roadmap bets clearly in docs and positioning |
| AI-native framing alienates developers who are not using agents heavily | Low | Keep the story grounded in trust, correctness, and clarity, which help all developers |
| Competing stacks copy the language-level story faster than ntnt can ship the workflow advantages | Medium | Focus on integrated product experience, not just syntax or isolated features |

## Alternatives Considered

### 1. Do nothing and let the existing roadmap imply this direction
Rejected because the shift in developer needs is significant enough that implicit alignment is not enough. Without an explicit roadmap layer, ntnt may miss both product opportunities and messaging clarity.

### 2. Focus only on language-level strictness
Rejected because the thread and article both suggest that whole-workflow usability matters more than raw strictness. ntnt should not become a language that is theoretically safer but practically harder to ship with.

### 3. Position ntnt as a direct Python replacement
Rejected because that invites a broad ecosystem fight that ntnt does not need. The stronger wedge is trustworthy AI-assisted app building, not general-purpose incumbency warfare.

## Definition of Done
- [ ] All implementation checklist items checked
- [ ] A reviewed roadmap proposal exists that translates this strategy into concrete ntnt work
- [ ] The proposed positioning is clear enough to test externally
- [ ] The most important roadmap bets are prioritized against existing ntnt work
- [ ] Design doc status updated appropriately
