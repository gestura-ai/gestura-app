# Electrical Engineering: Signals & Validation

Use this module when the task is analyzing signal integrity, planning board bring-up, defining measurement strategy, or building a validation plan that can expose intermittent faults.

## Focus

- Measurement access and debug hooks are part of the design, not optional extras.
- Signal-quality questions must consider timing, coupling, return paths, and instrumentation limits.
- Validation quality depends on bring-up sequence, expected waveforms, and fault-isolation discipline.

## High-Value Defaults

- Plan for measurement access and debug hooks from the start.
- Flag timing, noise, and coupling risks explicitly.
- Distinguish analog assumptions from digital assumptions.
- Include bring-up sequence, instrumentation, and fault isolation strategy.
- Prefer validation plans that catch intermittent failures early.

## Review Lens

- What should be measured first to establish board health?
- Which nets or domains are most sensitive to noise, skew, or crosstalk?
- Are the expected waveforms and timing margins documented?
- How will intermittent or temperature-dependent faults be isolated?

## Common Failure Modes

- Skipping test points and later struggling to debug the board.
- Treating analog and digital validation as if they need the same evidence.
- Running bring-up without a clear sequence or expected signatures.
- Ignoring intermittent faults until late system testing.

## Good Outputs

- Bring-up plans with pass/fail checkpoints.
- Measurement maps and required instrumentation.
- Signal-integrity risk reviews.
- Fault-isolation strategies for noisy or intermittent behavior.

## Retrieval Hints

signal integrity, board bring-up, debug hooks, instrumentation, fault isolation, crosstalk, timing margin, expected waveform, intermittent failure.

## Authoritative Sources

- IEEE
- IPC
- Analog Devices technical resources