# Robotics: Perception & Integration

Use this module when the task is about sensing, localization, calibration, synchronization, or the integration path between perception systems and the rest of a robot stack.

## Focus

- Perception quality depends on calibration, timing, and environment as much as on algorithms.
- Integration reliability is distinct from nominal model accuracy.
- Sensor and estimator interfaces should support degraded operation and postmortem debugging.

## High-Value Defaults

- Treat sensors, calibration, timing, and environmental assumptions explicitly.
- Separate perception accuracy from system integration reliability.
- Plan simulation, bench validation, and field validation in stages.
- Include data flow between sensors, estimators, and higher-level logic.
- Prefer architectures that degrade gracefully under sensor failure.

## Review Lens

- What environmental conditions break sensing assumptions?
- Are timestamps, clocks, and latency budgets reliable enough for fusion?
- How is calibration verified, refreshed, and monitored over time?
- What fallback behavior exists when confidence drops sharply?

## Common Failure Modes

- Assuming benchmark perception performance will transfer directly to the field.
- Fusing stale or misaligned sensor streams.
- Ignoring calibration drift after shock, wear, or temperature changes.
- Shipping without telemetry to reconstruct false positives and false negatives.

## Good Outputs

- Sensor and estimator interface maps.
- Calibration and synchronization review notes.
- Validation plans spanning sim, bench, HIL, and field.
- Degraded-mode strategies for partial sensor loss.

## Retrieval Hints

robot perception, localization, calibration, sensor fusion, timestamp alignment, SLAM, latency budget, degraded sensing, environment assumptions.

## Authoritative Sources

- ROS documentation
- Modern Robotics
- Probabilistic Robotics