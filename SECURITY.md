# Security policy

Slumber is a per-user shell execution tool, not a sandbox or a privilege boundary against programs running under the same OS account. Use a dedicated OS account or VM for untrusted work. Commands and resume templates have your account's filesystem and network permissions. Run without sudo.

## Implemented boundaries

- Before sending a request, the client checks socket ownership and the connected peer UID against the effective user ID. The daemon checks client UID before parsing requests.
- State and socket parents must be private, user-owned directories. Existing shared directories are never chmodded. IPC requests are size-limited with a read deadline.
- A process-held file lock serializes daemon startup and recovery. Metadata and environment snapshots are atomically replaced using private files.
- Job IDs are restricted before forming paths or remote shell strings. Submitted commands remain intentional shell code. SSH destinations are validated and SSH host-key checking is not disabled.
- Child processes receive the request environment, not the long-lived daemon environment. Successful wake-up is recorded before clearing the snapshot; startup repeats cleanup after an interrupted cleanup.

## Data handling and limits

`request.json` holds the full submitting environment in plaintext (`0600`, parent `0700`) until successful wake-up. Failed wake-ups retain it for explicit retry. `meta.json` contains commands, working directories and session identifiers. Task logs and `resume.log` can contain anything those programs print, including secrets. Daemon diagnostics may also include remote stderr. No automatic log expiration, encryption, secure erase, or backup exclusion is provided. File deletion/clearing does not erase backups or snapshots.

Treat job output and completion payloads as untrusted task data when interpreting them with an agent. Slumber does not filter prompt injection or constrain what an agent subsequently does. Same-user processes and root can read private state and impersonate trusted local programs; UID checks do not prevent that.

Local process groups are cleanup aids, not cgroups or a sandbox. Local work loses supervision on daemon crash. Remote work depends on a live local monitor, valid SSH credentials, intact remote logs and exit-code files. Delivery is not exactly-once. An ambiguous SSH launch error may leave a running remote job. These limits are part of the MVP support contract.

## Reporting

Use the repository's **Security → Report a vulnerability** form once private vulnerability reporting is enabled. If it is unavailable, request a private reporting channel from the maintainer without disclosing exploit details or credentials publicly. Do not attach `request.json`, raw environment dumps, private SSH configuration, or unredacted logs.

Include the Slumber version, OS/architecture, minimal reproduction and impact. Rotate any exposed credentials immediately. Only the most recent release will receive security fixes during the initial MVP period; no response-time SLA is promised.

CI runs dependency advisory checks and a redacted Git history secret scan on changes and weekly. Passing these checks is useful evidence, not a guarantee of absence of security bugs. Public release requires a successful fresh run and a review of its findings.
