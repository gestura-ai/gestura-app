# Headless / one-shot execution (`gestura exec`)

This template describes how to run a **single prompt** through Gestura in a way that is safe, reproducible, and automation-friendly.

## Preconditions

- You have a configured LLM provider in `~/.gestura/config.json` (see `docs/CONFIGURATION.md`).
- If your workflow requires machine-readable output, use the global `--json` flag.

## Input sources (prompt text)

`gestura exec` accepts prompt text from exactly one of:

1) **Positional argument** (prompt inline)
2) **`--file`** (prompt loaded from a text file)
3) **stdin** (when input is piped; non-interactive)

If none are provided, `gestura exec` fails with a clear error.

## Reproducibility guidance

- Prefer **`--file`** or **stdin** for longer prompts so the exact input is versionable/auditable.
- Keep the working directory explicit in scripts.
- Avoid including secrets in the prompt text.

## Model selection

`gestura exec` supports `--model` overrides.

- You may specify a provider-qualified model string using the form `provider:model`.
- Current behavior updates the model for OpenAI/Anthropic provider configs when specified.

## Minimal “contract” check

Before relying on this in automation, verify:

- `gestura exec --help` documents expected args/flags
- `gestura exec` returns a non-zero exit code on missing/empty prompt input

## Failure handling

- If the command fails, treat stderr as the primary signal.
- If the failure is provider-related, validate:
  - provider selection in `~/.gestura/config.json`
  - network access (if using remote providers)
  - key availability (never paste keys into logs)

