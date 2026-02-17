# Sales: Pipeline Review

**Derived from:** `anthropics/knowledge-work-plugins` (`sales/commands/pipeline-review.md`)  
**License:** Apache-2.0  
**Source URL:** https://raw.githubusercontent.com/anthropics/knowledge-work-plugins/main/sales/commands/pipeline-review.md  
**Modifications:** condensed + reformatted for Gestura’s built-in knowledge.

## Goal
Assess pipeline health and produce:
- top weekly priorities
- deal risk flags (stale, stuck, slipped close)
- hygiene issues
- a practical action plan

## Inputs
One of:
- CSV export (deal, amount, stage, close date)
- pasted deal list with last activity
- quick summary by stage

Helpful fields:
- created date, last activity, primary contact, owner, segment

## Priority framework (default weights)
- Close date urgency (30%)
- Deal size (25%)
- Stage (20%)
- Activity recency (15%)
- Risk (10%)

## Checks
- **Stale**: no activity 14+ days
- **Stuck**: same stage 30+ days
- **Slipped**: close date in the past
- **Single-threaded**: only one contact / champion
- **Missing basics**: no amount, no next step, no close date

## Output (markdown)
```markdown
# Pipeline Review: [Date]

**Deals Analyzed:** [N]
**Total Pipeline:** $[X]

## Pipeline Health Score: [X/100]
| Dimension | Score | Note |
|---|---:|---|
| Stage progression | [X]/25 | ... |
| Activity recency | [X]/25 | ... |
| Close date accuracy | [X]/25 | ... |
| Contact coverage | [X]/25 | ... |

## Priority actions this week
1. **[Deal]** — [Why] → [Next action]
2. ...

## Risk flags
### Stale (14+ days)
| Deal | Amount | Last activity | Recommendation |
|---|---:|---|---|

### Stuck (30+ days)
| Deal | Amount | Stage | Days in stage | Recommendation |
|---|---:|---|---:|---|

### Slipped close dates
| Deal | Amount | Close date | Recommendation |
|---|---:|---|---|

## Hygiene
| Issue | Count | Action |
|---|---:|---|

## Suggested weekly plan
1. ...
```

## If CRM is connected
- pull real-time opportunities + activities
- suggest stage moves / close date updates (with confirmation)
- create tasks for the weekly plan
