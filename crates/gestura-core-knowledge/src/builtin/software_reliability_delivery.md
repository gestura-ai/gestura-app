# Software: Reliability & Delivery

Use this module when the task is planning a rollout, defining verification strategy, improving observability, or reducing the chance and impact of production incidents.

## Focus

- Delivery plans should pair rollout with rollback and detection.
- Verification should be layered according to risk, not habit.
- Reliability work should improve diagnosability as well as uptime.

## High-Value Defaults

- Define how success, failure, and regressions will be detected.
- Prefer layered verification: unit, integration, end-to-end, and production signals.
- Design deployment and rollback together.
- Include logging, metrics, tracing, and alerting expectations.
- End with the safest implementation and rollout path.

## Review Lens

- What is the blast radius if this change fails?
- Which tests protect the highest-risk behavior?
- How quickly would operators detect and localize a regression?
- What recovery path exists under time pressure?

## Common Failure Modes

- Shipping with no clear production success signal.
- Relying on a single test layer to catch all regressions.
- Rolling out changes that cannot be mitigated or rolled back safely.
- Emitting telemetry that is too shallow to diagnose incidents.

## Good Outputs

- Rollout plans with gates, owners, and rollback triggers.
- Verification matrices tied to risk.
- Observability plans for logs, metrics, traces, and alerts.
- Reliability reviews with likely incidents and mitigations.

## Retrieval Hints

reliability, rollout plan, rollback, observability, tracing, alerting, verification strategy, feature flag, blast radius, regression detection.

## Authoritative Sources

- Google SRE Book
- AWS Well-Architected
- NIST Secure Software Development Framework