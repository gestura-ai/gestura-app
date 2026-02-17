# Sales: Call Prep

**Derived from:** `anthropics/knowledge-work-plugins` (`sales/skills/call-prep/SKILL.md`)  
**License:** Apache-2.0  
**Source URL:** https://raw.githubusercontent.com/anthropics/knowledge-work-plugins/main/sales/skills/call-prep/SKILL.md  
**Modifications:** condensed + reformatted for Gestura’s built-in knowledge.

## Goal
Generate a call prep brief: attendee context, agenda, discovery questions, and likely objections.

## Trigger phrases
- “prep me for my call with [company]”
- “get me ready for [meeting]”
- “call prep [company]”

## Inputs
Required:
- company/contact
- meeting type (discovery/demo/negotiation/check-in)

Helpful:
- attendees (names/titles)
- prior notes/emails/transcripts
- desired outcome (what success looks like)

## Steps
1. Pull what’s available (calendar/CRM/email/transcripts) if connected
2. Run quick web research for company + attendees
3. Identify:
   - deal context + stage
   - the 3–5 must-answer questions
   - risks/objections likely at this stage
4. Produce a structured brief with a recommended next step

## Output brief (markdown)
```markdown
# Call Prep: [Company]

**Meeting:** [Type] — [Date/Time]
**Attendees:** [Names]
**Your goal:** [Outcome]

## Account snapshot
| Field | Value |
|---|---|
| Industry | ... |
| Size | ... |
| Status | ... |
| Last touch | ... |

## Who you’re meeting
- [Name] — [Role] — [Talking point]

## Suggested agenda
1. Open + context
2. Discovery: priorities + current state
3. Deep dive: [topic]
4. Objections/risks
5. Next steps (timeboxed)

## Discovery questions
1. ...

## Likely objections + responses
| Objection | Response |
|---|---|

## After the call
- Run call summary + follow-up
```
