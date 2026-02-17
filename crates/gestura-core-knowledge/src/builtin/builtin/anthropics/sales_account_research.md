# Sales: Account Research

**Derived from:** `anthropics/knowledge-work-plugins` (`sales/skills/account-research/SKILL.md`)  
**License:** Apache-2.0  
**Source URL:** https://raw.githubusercontent.com/anthropics/knowledge-work-plugins/main/sales/skills/account-research/SKILL.md  
**Modifications:** condensed + reformatted for Gestura’s built-in knowledge.

## What to do
Research a company/person and produce actionable sales intel:
- what they do + positioning
- recent news + triggers
- leadership + key stakeholders
- hiring signals
- suggested outreach angles + discovery questions

## Triggers
- “research [company]”
- “intel on [prospect]”
- “who is [person] at [company]”
- “tell me about [company] before my call”

## Steps
1. **Disambiguate target** (company vs person; accept domain)
2. **Web research pass** (always):
   - company homepage/about
   - news (last 90 days)
   - funding (if relevant)
   - careers page + notable roles
   - exec/team pages
3. **If available: enrichment** (tech stack, org chart, contacts)
4. **If available: CRM** (relationship history, opps, activities)
5. **Synthesize** into a concise brief + recommended approach

## Output format (markdown)
```markdown
# Research: [Company or Person]
**Generated:** [Date]
**Sources:** Web [+ Enrichment] [+ CRM]

## Quick take
[2–3 sentences: who they are + best outreach angle]

## Company profile
| Field | Value |
|---|---|
| Website | ... |
| Industry | ... |
| Size | ... |
| HQ | ... |
| Founded | ... |

## Recent news / triggers
- [Headline] — [Date] — [Why it matters]

## Key people
- [Name] — [Title] — [Why they matter]

## Qualification signals
**Positive:** ...
**Concerns:** ...
**Unknowns (ask):** ...

## Recommended approach
**Best entry point:** ...
**Hook:** ...
**Discovery questions:**
1. ...

## Sources
- [Link](URL)
```
