# Aerospace: Guidance & Verification

Use this module when the task is analyzing GNC behavior, defining validation strategy, planning qualification, or identifying the next critical verification step for an aerospace system.

## Focus

- Guidance, navigation, and control claims should expose assumptions about stability, disturbance rejection, and fault handling.
- Verification should be staged across analysis, simulation, hardware test, and qualification evidence.
- Safety margins and failure containment are as important as nominal performance.

## High-Value Defaults

- Make stability, guidance, and fault handling assumptions explicit.
- Plan verification as analysis, simulation, test, and qualification stages.
- Keep safety margins and failure containment visible.
- Distinguish conceptual advice from certifiable design decisions.
- End with the critical verification step that should happen next.

## Review Lens

- What assumptions about sensors, actuators, disturbances, and environment underpin the controller?
- Which failure cases must be contained before flight confidence is justified?
- Does the verification chain produce evidence that is relevant to qualification?
- What single verification gap currently dominates the risk picture?

## Common Failure Modes

- Tuning GNC behavior without surfacing disturbance or fault assumptions.
- Treating simulation as sufficient proof of flight readiness.
- Ignoring margin erosion under off-nominal conditions.
- Confusing conceptual stability arguments with certifiable evidence.

## Good Outputs

- GNC assumption and stability summaries.
- Verification roadmaps spanning analysis through qualification.
- Safety-margin and fault-containment reviews.
- Recommendations for the next critical verification activity.

## Retrieval Hints

guidance, navigation, control, GNC, qualification, verification, stability margin, fault containment, off-nominal condition, flight readiness.

## Authoritative Sources

- NASA Systems Engineering Handbook
- FAA guidance
- ESA ECSS standards