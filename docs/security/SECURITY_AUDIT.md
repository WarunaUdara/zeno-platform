# Security audit — 2026-10-21

This audit was run before the repository was published, using the
[Cloudflare security-audit skill](https://github.com/cloudflare/security-audit-skill)
as the review method: reconnaissance, coverage-led hunting by trust boundary,
candidate validation, and remediation. The review was source-first and read-only;
no live endpoint was probed and nothing was exploited.

## Scope

| Area | Reviewed |
|---|---|
| API authentication and authorization | JWT verification, JWKS handling, organization membership checks, role gates, server-to-server identity |
| Tenant isolation | Event, ticket, order, check-in, attendance, and cover-image access across organizations |
| Input handling | Reservation and confirmation input, form answers, file uploads, image decoding, QR credentials, path construction |
| Cryptography | Ticket credential sealing, hashing, idempotency, replay handling |
| Data handling | Error responses, logs and traces, metrics labels, client bundles |
| CI and supply chain | GitHub Actions, Workload Identity Federation, IAM, Dockerfiles, dependency configuration |
| Secrets | Full git history and working tree, including ignored local files |

## Result

The published tree contains no credentials. The local, ignored `.env` used during
development was never tracked, and value-level searches for its contents across
all reachable git history returned no matches. The public copy was created from
the tracked tree, so untracked local state (`.env`, `.terraform`, build output)
was not carried over. The new repository starts from a single commit that uses a
GitHub noreply author address rather than a personal email.

Eleven candidates were raised and validated. Six crossed a real trust boundary
or weakened a stated control; all six are fixed. The remaining five are hardening
notes or deployment-dependent gaps and are listed as residual risks below.

## Fixed

### 1. Pre-authentication JWKS amplification

`apps/api/src/auth.rs` took the `kid` from an unverified token header and fetched
the JWKS whenever that key was absent, while holding the cache mutex across the
network call. An unauthenticated caller could send random key ids and serialize
every request behind a provider fetch.

The cache now stores keys, expiry, and the last fetch attempt. A refresh is
claimed once per cooldown window, the mutex is released before network I/O, and
a provider failure serves the last known key set rather than invalidating it.
Concurrent callers observe the claim and use the cached set. Unknown key ids
inside the cooldown fail verification immediately instead of causing a fetch.
Regression tests assert both cache reuse and that an unknown key id does not
re-fetch.

### 2. Cross-tenant check-in replay

`apps/api/src/checkin.rs` looked up a replayed `scanner_action_id` globally, before
the caller's organization and event were verified. The ledger's uniqueness is
global, so a member of one organization could reuse another organization's action
id and receive that ticket's outcome, including the attendee name.

The replay lookup and the ticket read are now scoped by `organization_id` and
`event_id`. A cross-tenant action id is treated as a fresh scan. A regression test
creates two organizations, reuses the action id, and asserts the second scan is
neither marked as a replay nor discloses the first ticket.

### 3. Reservation id exposure in traces

`apps/api/src/orders.rs` recorded the full reservation id as a span field. A
reservation id is a bearer capability for order retrieval, so a trace viewer
could replay one against the confirm endpoint.

The span now records an eight-character reference only, and the replay query also
requires the event id, so a reservation id cannot be used against a different
event even if it leaks.

### 4. Workload Identity Federation was repository-bound only

`infra/gcp/cicd.tf` pinned the repository claim but not the ref or environment.
A `pull_request` run or a `workflow_dispatch` on a branch carries the base
repository claim, so a collaborator-pushed branch could mint deploy credentials.

The provider condition now requires all three claims: the repository, a push to
`refs/heads/main`, and the `production` environment. The repository input is also
validated with a strict owner/name pattern. The deploy workflow is additionally
gated on `github.ref == 'refs/heads/main'`, has a job timeout, checks out with
`persist-credentials: false`, and writes no credentials file.

### 5. Mutable container base images

Both Dockerfiles referenced mutable tags. They now pin the multi-architecture
image digests for the Rust build, Debian runtime, Bun build, and Node runtime
stages. Dependabot already tracks both Dockerfiles, so digest updates remain
reviewable pull requests.

### 6. Unthrottled confirmation and permissive edge default

The Cloud Armor policy throttled `POST /api/reservations` but not
`POST /api/reservations/confirm`, which is unauthenticated and issues tickets.
The policy now covers both paths. The `public_reservation_rate_limit_preview`
variable defaulted to `true`, which logs matches without enforcing them; the
default is now `false`, and the example tfvars matches.

Additional changes from the same review: `.dockerignore` excludes nested
environment files and `gha-creds-*.json`; CI runs gitleaks on every pull request
and push; deploy action versions are pinned to commit SHAs; and the deployment
workflow skips cleanly until its repository variables are configured.

## Residual risks

These are documented rather than silently accepted.

- **The deploy identity can read secrets indirectly.** `roles/run.developer`
  permits deploying a revision that references an existing secret, even though
  `roles/secretmanager.secretAccessor` is not granted on the Secret Manager
  resources. A fully separate deploy identity requires a two-stage pipeline or a
  custom IAM condition. For a single-maintainer project, the compensating control
  is that only a push to `main` in the `production` environment can use the
  identity.
- **`/metrics` is unauthenticated.** The API service is only invokable by the web
  service account, so this is not internet-reachable in the reference topology.
  Do not expose the API directly without fronting metrics with authentication.
- **JWT audience is not validated.** Neon Auth access tokens are validated for
  signature, issuer, expiry, and algorithm. Audience validation depends on the
  provider configuration and is left to the deployment.
- **A reservation id remains a bearer capability.** It is random and short-lived,
  but anyone holding it can read the order. Do not put it in referrers, analytics,
  or support tooling.
- **Terraform state is local by default.** Use an encrypted remote backend with
  locking before any shared deployment. State can contain sensitive values.

## Verification

Executed on the published tree:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
DATABASE_URL=postgres://zeno:zeno_dev_only@127.0.0.1:5432/zeno cargo test --workspace
bun install --frozen-lockfile
bun run api:lint
bun run --cwd apps/web test
bun run --cwd apps/web lint
bun run --cwd apps/web typecheck
bun run --cwd apps/web build
tofu -chdir=infra/gcp fmt -check -recursive
tofu -chdir=infra/gcp validate -no-color
tofu -chdir=infra/gcp test -no-color
gitleaks dir --config .gitleaks.toml .
```

All passed. The database integration tests require a local PostgreSQL; they
refuse to run against a non-loopback host.

## Reporting

See [SECURITY.md](../../SECURITY.md). Do not open a public issue for a suspected
vulnerability.
