# Sales: Forecast

**Derived from:** `anthropics/knowledge-work-plugins` (`sales/commands/forecast.md`)  
**License:** Apache-2.0  
**Source URL:** https://raw.githubusercontent.com/anthropics/knowledge-work-plugins/main/sales/commands/forecast.md  
**Modifications:** condensed + reformatted for Gestura’s built-in knowledge.

## Goal
Generate a weighted forecast with:
- best / likely / worst scenarios
- commit vs upside breakdown
- gap-to-quota analysis + recommendations

## Inputs
### Pipeline data (one of)
- CSV export (deal, amount, stage, close date)
- pasted deal list
- quick summary (counts + totals per stage)

### Targets
- quota for the period
- period end date
- closed-to-date amount

Optional (improves quality):
- owner/rep (team forecasting)
- last activity date per deal
- segment, deal size, probability overrides

## Default stage probabilities (override if provided)
- Negotiation / Contract: 80%
- Proposal / Quote: 60%
- Evaluation / Demo: 40%
- Discovery / Qualification: 20%
- Prospecting / Lead: 10%

## Steps
1. Clean/validate pipeline rows (missing amounts/dates)
2. Map stage → probability (use custom if provided)
3. Compute weighted value per deal + aggregate
4. Build scenarios:
   - **Commit**: high-confidence subset
   - **Upside**: plausible but riskier subset
   - **Worst**: commit only (or strict subset)
5. Flag risks (stale activity, slipped close dates, stage/date mismatch)
6. Recommend actions to close the gap

## Output (markdown)
```markdown
# Sales Forecast: [Period]

**Generated:** [Date]
**Data Source:** [CSV / Manual / CRM]

## Summary
| Metric | Value |
|---|---|
| Quota | $[X] |
| Closed to Date | $[X] ([X]%) |
| Open Pipeline | $[X] |
| Weighted Forecast | $[X] |
| Gap to Quota | $[X] |
| Coverage Ratio | [X]x |

## Scenarios
| Scenario | Amount | % of Quota | Assumptions |
|---|---:|---:|---|
| Best | $[X] | [X]% | ... |
| Likely | $[X] | [X]% | ... |
| Worst | $[X] | [X]% | ... |

## Commit vs Upside
### Commit (High confidence)
| Deal | Amount | Stage | Close Date | Why commit |
|---|---:|---|---|---|

### Upside (Lower confidence)
| Deal | Amount | Stage | Close Date | Primary risk |
|---|---:|---|---|---|

## Risk flags
| Deal | Risk | Recommendation |
|---|---|---|

## Recommendations
1. ...
```

## If CRM/activity data is available
- replace default probabilities with historical win rates
- add activity-based risk scoring (recency, meetings, champions)
- track deltas week-over-week
