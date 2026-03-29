# Analytics: Metrics & Instrumentation

Use this module when the task is defining KPIs, designing event tracking, building a metric tree, or deciding how analytics data should be instrumented and governed.

## Focus

- Metrics should be designed around operating decisions, not dashboard aesthetics.
- Instrumentation should capture the behavior that matters with clear ownership and reproducible semantics.
- Data quality, identity resolution, and timing reliability determine whether downstream analysis can be trusted.

## High-Value Defaults

- Start from the decision to be made, then define the smallest trustworthy metric set.
- Separate north-star, driver, diagnostic, and guardrail metrics explicitly.
- Define the analysis entity first: user, account, session, order, workspace, or cohort.
- Specify event names, required properties, timestamps, ownership, and lag before analysis begins.
- Prefer reproducible metric definitions over dashboard-specific logic or one-off calculations.

## Review Lens

- Is the metric definition implementable consistently across product, warehouse, and BI layers?
- Are inclusion criteria, exclusions, windows, and attribution rules explicit?
- Can instrumentation distinguish user intent from technical noise or retries?
- What source of truth governs each entity and event timestamp?

## Common Failure Modes

- Building dashboards before agreeing on what the KPI actually means.
- Reusing event names for different user intents.
- Mixing backend facts and frontend events without reconciliation logic.
- Allowing identity stitching, schema drift, or late-arriving events to silently distort reporting.

## Good Outputs

- KPI definitions with formulas, windows, and exclusions.
- Event dictionaries with required properties and ownership.
- Tracking plans tied to product flows and operating reviews.
- Data-quality risk lists with validation checks.

## Retrieval Hints

metric tree, KPI definition, tracking plan, event taxonomy, instrumentation, source of truth, attribution, identity stitching, schema drift, dashboard inputs.

## Authoritative Sources

- Amplitude documentation
- Google Analytics documentation
- NIST Engineering Statistics Handbook