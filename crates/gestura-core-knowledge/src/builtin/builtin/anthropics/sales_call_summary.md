# Sales: Call Summary

**Derived from:** `anthropics/knowledge-work-plugins` (`sales/commands/call-summary.md`)  
**License:** Apache-2.0  
**Source URL:** https://raw.githubusercontent.com/anthropics/knowledge-work-plugins/main/sales/commands/call-summary.md  
**Modifications:** condensed + reformatted for Gestura’s built-in knowledge.

## What this is
Turn raw call notes or a transcript into:
- an internal summary
- actionable next steps (owners + due dates)
- a customer follow-up email draft

## When to use (triggers)
- “summarize this call” / “call recap” / “what are the action items?”
- “draft the follow-up email”
- after discovery/demo/negotiation calls

## Inputs to ask for
Minimum:
- Notes or transcript (paste whatever you have)

Helpful:
- Company + deal stage
- Attendees (names/titles)
- Any open risks/objections you noticed

## Workflow
1. **Normalize the input** (remove filler, keep timestamps if present)
2. **Extract decisions + commitments** (what was agreed)
3. **Generate action items** (owner, due date if implied)
4. **Surface risks / open questions** (explicitly list them)
5. **Draft follow-up email** (plain text, concise, clear CTA)

## Output templates

### Internal summary (markdown)
```markdown
## Call Summary: [Company] — [Date]

**Attendees:** [Names + titles]
**Call Type:** [Discovery / Demo / Negotiation / Check-in]
**Duration:** [If known]

### Key Discussion Points
1. ...

### Customer Priorities
- ...

### Objections / Concerns Raised
- ...

### Competitive Intel
- ...

### Action Items
| Owner | Action | Due |
|-------|--------|-----|
| ...   | ...    | ... |

### Next Steps
- ...

### Deal Impact
- [Stage change / risk / acceleration]
```

### Customer follow-up email (plain text)
Subject: [Recap + next steps]

Hi [Name],

Thanks again for your time today. Here’s a quick recap of what we covered:

- [Point 1]
- [Point 2]

Next steps:
- [Their action] — [date]
- [Our action] — [date]

Does [day/time] work for [next meeting / technical deep dive]?

Best,
[You]

## Style rules for customer email
- Plain text (no markdown bolding)
- 2–3 sentence paragraphs
- One clear CTA (timeboxed)

## If connectors/tools are available
- Pull transcript automatically (e.g., Gong/Fireflies)
- Log call summary + next steps into CRM
- Create tasks/reminders
- Create an email draft in the user’s mailbox
