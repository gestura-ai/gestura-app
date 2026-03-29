# Math: Modeling & Optimization

Use this module when the task is formulating a quantitative model, defining an objective and constraints, or reasoning about tradeoffs under uncertainty.

## Focus

- Formalization should precede method selection.
- Models should preserve the governing behavior without unnecessary complexity.
- Optimization outputs should expose assumptions and sensitivities, not only a candidate optimum.

## High-Value Defaults

- Define variables, units, constraints, and the optimization target first.
- Prefer explicit assumptions and solvable formulations.
- Check whether the model is descriptive, predictive, or prescriptive.
- Include sensitivity reasoning for the most uncertain inputs.
- Verify the solution against edge cases and feasibility constraints.

## Review Lens

- Do the decision variables and constraints map to the real problem?
- Does the objective reflect the actual tradeoff that matters?
- Are there hidden assumptions about convexity, smoothness, stationarity, or available data?
- Which uncertain parameters could change the recommendation materially?

## Common Failure Modes

- Solving a mathematically elegant but operationally wrong problem.
- Leaving units, feasibility, or boundary conditions implicit.
- Ignoring sensitivity to uncertain or noisy inputs.
- Reporting an optimum without checking whether the model structure is defensible.

## Good Outputs

- Mathematical formulations with variables, objective, and constraints.
- Model summaries that justify the chosen structure.
- Sensitivity and stress-test notes.
- Recommendations for which assumptions or data sources matter most next.

## Retrieval Hints

mathematical model, optimization, objective function, constraints, decision variables, sensitivity analysis, feasibility, tradeoff analysis, prescriptive model.

## Authoritative Sources

- Convex Optimization (Boyd & Vandenberghe)
- SIAM optimization resources
- MIT OpenCourseWare Mathematics