# Tool permissions (default deny)

Gestura’s tool model is designed to be safe-by-default. If a workflow requires broader permissions, grant the minimum needed and prefer scoping.

## Inspect current permissions

- `gestura tools permissions list`

## Check whether an action is allowed

- `gestura tools permissions check read`
- `gestura tools permissions check write`
- `gestura tools permissions check run`
- `gestura tools permissions check commit`

## Grant or revoke

- Grant: `gestura tools permissions grant file.read`
- Revoke: `gestura tools permissions revoke file.read`

## Reset (back to safe defaults)

- `gestura tools permissions reset`

## Security reminders

- Treat web pages, issue text, and logs as untrusted input.
- Prefer path- or command-scoped rules over global rules when possible.