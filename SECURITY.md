# Security policy

## Reporting a vulnerability

Please do not open a public issue for a security problem.

Use GitHub's private vulnerability reporting on this repository: open the
**Security** tab and choose **Report a vulnerability**. That channel is visible
only to the maintainer and to you, and it produces a private advisory where the
fix can be discussed before anything is published.

If that channel is unavailable to you, open a public issue that says only that
you have a security report and asks for a private channel. Do not include the
details.

Please include what the problem is, how to reproduce it, and what an attacker
gains. A working reproduction against a local stack is the most useful thing you
can send.

## Audit history

The repository was reviewed before publication. See
[`docs/security/SECURITY_AUDIT.md`](docs/security/SECURITY_AUDIT.md) for the
scope, the issues that were fixed, and the residual risks.

## What to expect

| Stage | Target |
|---|---|
| Acknowledgement | 3 days |
| Initial assessment | 7 days |
| Fix for a confirmed high-severity issue | As fast as is practical; this is a single-maintainer project |

This is a single-maintainer open-source project, not a funded product with an
on-call rotation. Those are honest targets, not a service-level agreement.

## Scope

In scope: this repository's code, and the configuration in `infra/`.

Out of scope: findings that require a compromised developer machine, denial of
service by volume alone, missing hardening headers with no demonstrated impact,
and automated scanner output with no reproduction.

## What this project already assumes

Stating the design intent makes it easier to judge whether something is a bug.

- **Ticket credentials are bearer secrets.** A QR code proves the issuer created
  the payload. It does not prove the presenter is the owner, and possession of
  the credential is possession of the ticket. Admission is protected by an atomic
  status transition, so a copied credential is detected on the second scan, not
  rejected on the first.
- **Phone cameras are trusted endpoints.** The API never authorises a check-in on
  the strength of a scan alone; the scanner is an authenticated organizer
  session with permission on the event.
- **The API is not publicly reachable in the deployed environment.** The web tier
  calls it with a Cloud Run identity token. A finding that assumes direct API
  access should say how that access was obtained.
- **Secrets live outside the repository.** Locally in `.env`, which is gitignored,
  and in GCP in Secret Manager, referenced by pinned version.
- **Least privilege is enforced by the database.** The runtime connects as a
  DML-only role; schema changes run under a separate owner credential that only
  the migration job mounts. See
  [`docs/operations/SECRET_ROTATION.md`](docs/operations/SECRET_ROTATION.md).

## Scanning

The repository ships a secret-scanning configuration, so the check is repeatable
rather than remembered:

```sh
gitleaks git --config .gitleaks.toml --redact
```

The allowlist in that file is short and every entry says why it grants no remote
access. If you are reviewing this project, that file is a good place to check
whether the security posture is what it claims.

## Supported versions

Pre-1.0. Only the `main` branch is supported, and there are no tagged releases
yet. A fix lands on `main`; there are no backports.
