# Intent-First Architecture

## Purpose

Gestura.app is an intent-first AI assistant. The current implementation accepts user intent from voice, chat, Haptic Harmony ring gestures, and future input adapters, then routes each request through a shared core execution model.

## Core Principle

The product is organized around **intent**, not around any single modality. Voice, text, and gesture capture remain important entry points, but they do not define separate execution systems. Each modality produces a normalized intent that can be resolved, acted on, verified, and observed through the same core loop.

## Unified Intent Normalization Layer

The normalization layer sits between modality capture and execution.

1. **Capture** — GUI, CLI, voice pipelines, chat surfaces, and ring integrations collect user input in their native form.
2. **Normalize** — Each adapter translates that input into a shared intent shape with common action, context, and confidence metadata.
3. **Resolve** — The core stack applies permissions, guardrails, context resolution, and tool policy to the normalized intent.
4. **Execute** — The same agentic loop performs direct actions or tool-assisted multi-step work.
5. **Respond** — Results return through the originating surface, with optional visual, textual, or haptic feedback.

This preserves a single source of truth in `gestura-core` while keeping modality-specific code focused on capture and presentation.

## How the Core Loop Preserves the Existing Workflow

The intent-first model does not replace the established workflow. It preserves the original Core-First approach and clarifies where modality-specific behavior belongs.

- **GUI and CLI remain thin presentation layers**.
- **`gestura-core` remains the shared business-logic surface**.
- **Policy, permissions, context, tools, and streaming stay centralized**.
- **Voice, chat, and ring gesture adapters all converge before execution begins**.

In practice, the workflow remains: capture input → normalize intent → resolve context/policy → execute → verify if needed → return output.

## Optional Advanced Primitives in `gestura-core-tasks`

`gestura-core-tasks` now serves as the home for optional advanced primitives that can be attached when an intent becomes complex or explicitly multi-step.

- **TaskRegistry** manages durable task state, delegation metadata, and workflow ownership.
- **Verification loops** provide bounded plan/act/verify cycles for intents that need checking, retries, or structured completion criteria.
- **Semantic client** provides higher-level task and intent composition so complex work can be routed consistently across domains.

These primitives are **conditional middleware**, not mandatory overhead. Straightforward intents can continue through the standard loop, while complex intents can opt into richer coordination and verification behavior.

## Modality Support Model

| Modality | Role in the architecture | Result |
|----------|---------------------------|--------|
| Voice | Captures spoken requests and transcriptions | Normalized into the shared intent path |
| Chat | Captures typed or pasted requests | Normalized into the shared intent path |
| Haptic Harmony ring gestures | Captures gesture-driven requests and control signals | Normalized into the shared intent path |
| Future inputs | Adds new adapters without redefining the core loop | Reuses the same normalization and execution model |

## Operational Implications

- New input modalities should add adapters, not duplicate business logic.
- Shared observability should describe intent lifecycle, not just source-specific events.
- Advanced task coordination should remain optional and should activate only when complexity justifies it.
- Documentation and product positioning should describe Gestura.app as intent-first, modality-flexible, and approaching production stability.