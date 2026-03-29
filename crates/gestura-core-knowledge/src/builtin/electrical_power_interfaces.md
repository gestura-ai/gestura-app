# Electrical Engineering: Power & Interfaces

Use this module when the task is defining a power architecture, reviewing electrical interfaces, or reasoning about grounding, isolation, and block-level hardware risk.

## Focus

- Power, signaling, timing, and safety concerns should be separated explicitly.
- Interface quality depends on rails, return paths, grounding assumptions, and fault behavior.
- Early block-level clarity prevents board-level surprises later.

## High-Value Defaults

- Partition power, signal, timing, and safety concerns clearly.
- Define loads, rails, interfaces, and grounding assumptions first.
- Include derating, isolation, and thermal concerns where relevant.
- Prefer testable architectures over dense feature coupling.
- Call out block-level risks before board-level detail.

## Review Lens

- Are all loads, startup transients, and fault conditions captured?
- How do grounding and return paths affect interface behavior?
- What isolation, protection, or sequencing assumptions are safety-critical?
- Which block interactions could create thermal or power-integrity problems?

## Common Failure Modes

- Treating schematic correctness as equivalent to robust power behavior.
- Underestimating inrush, transient, or sequencing interactions.
- Leaving grounding and return-path assumptions implicit.
- Packing too much feature coupling into a design that is hard to validate.

## Good Outputs

- Block-level power architectures.
- Interface reviews with protection and grounding notes.
- Rail and load-budget summaries.
- Risk lists for isolation, thermal headroom, and fault behavior.

## Retrieval Hints

power architecture, electrical interface, grounding, isolation, rail budget, startup transient, protection, return path, derating, thermal headroom.

## Authoritative Sources

- IEEE
- IPC
- Texas Instruments technical documents