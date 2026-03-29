# Analytics: Experimentation & Insights

Use this module when the task is designing experiments, reading test results, explaining metric movement through cohorts or segmentation, or deciding what insight should change product or business action.

## Focus

- Separate causal claims from descriptive trend reporting.
- Use cohorts and segments to explain mechanism, not just movement.
- Every readout should produce a recommendation with confidence and uncertainty stated plainly.

## High-Value Defaults

- Define the hypothesis, success metric, guardrails, and decision threshold up front.
- Distinguish baseline trend analysis from causal experiment interpretation.
- Use cohorts and segments to explain *why* a metric moved, not only *that* it moved.
- State uncertainty, sample-size risk, novelty effects, survivorship bias, and instrumentation concerns explicitly.
- End with a recommendation, confidence level, and next measurement step.

## Review Lens

- Was eligibility, randomization, exposure, or assignment handled correctly?
- Are cohorts aligned by start point, maturity, or lifecycle stage?
- Does the observed effect matter operationally, not only statistically?
- What evidence would overturn the current recommendation?

## Common Failure Modes

- Treating pre/post movement as if it proved causality.
- Using too many post-hoc segments to rescue a weak result.
- Ignoring guardrail degradation because the primary metric improved.
- Explaining retention shifts without checking acquisition-mix or seasonality changes.

## Good Outputs

- Experiment briefs with hypotheses, thresholds, and decision rules.
- Readouts with effect size, uncertainty, and operational interpretation.
- Cohort analyses that distinguish maturity effects from intervention effects.
- Segmentation frameworks that guide the next investigation.

## Retrieval Hints

experiment design, A/B test, causal inference, cohort analysis, segmentation, readout, effect size, guardrail metric, novelty effect, survivorship bias.

## Authoritative Sources

- CXL experimentation resources
- NIST Engineering Statistics Handbook
- Amplitude experiment and analysis documentation