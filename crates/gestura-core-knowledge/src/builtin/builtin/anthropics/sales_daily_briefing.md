# Sales: Daily Briefing

**Derived from:** `anthropics/knowledge-work-plugins` (`sales/skills/daily-briefing/SKILL.md`)  
**License:** Apache-2.0  
**Source URL:** https://raw.githubusercontent.com/anthropics/knowledge-work-plugins/main/sales/skills/daily-briefing/SKILL.md  
**Modifications:** condensed + reformatted for Gestura’s built-in knowledge.

## Goal
Provide a fast, prioritized “start my day” brief:
- #1 priority
- today’s meetings + prep actions
- pipeline alerts
- email priorities
- 3 suggested actions

## Triggers
- “daily brief” / “morning briefing” / “what’s on my plate today?”

## Inputs
If no connectors:
- today’s meetings (paste calendar or list)
- key deals + what’s urgent

If connectors exist (calendar/CRM/email): pull automatically.

## Prioritization rules
1. Deal closing today/tomorrow not yet won
2. Meeting today with high-value opp
3. Unread email from decision-maker
4. Deals closing this week
5. Stale deals (no activity 7–14+ days)

## Output template
```markdown
# Daily Briefing | [Date]

## #1 Priority
**[Action]** — [Why it matters]

## Today’s meetings
- [Time] — [Company] — [Prep action]

## Pipeline alerts
- ...

## Email priorities
- ...

## Suggested actions
1. ...
2. ...
3. ...
```
