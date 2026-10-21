# Contributing

Thanks for looking. This document covers how to get the stack running, how to
verify a change, and what a reviewable commit looks like here.

Before you start, read the architecture decision records in [`docs/adr/`](docs/adr)
before proposing a structural change. The ADRs record what was deliberately
rejected, which is often more useful than what was chosen.

## Getting set up

Requirements: Docker, Bun, Rust (stable), and OpenTofu only if you are touching
infrastructure.

```sh
cp .env.example .env     # or .env.floci.example for the emulated-GCP path
bun install
bun run stack:up         # PostgreSQL, API, worker, web, Prometheus, Grafana
```

Grafana is at <http://127.0.0.1:3002>, Prometheus at <http://127.0.0.1:9090>, and
the web app at <http://127.0.0.1:3000>. The local Grafana credential is a
development default; set `GRAFANA_ADMIN_PASSWORD` to override it.

## Verifying a change

Run the checks that cover what you touched. The full set:

```sh
# API: unit and database integration tests
DATABASE_URL=postgres://zeno:zeno_dev_only@127.0.0.1:5432/zeno cargo test --workspace

# Web: tests, types, lint, production build
cd apps/web && bun run test && bun run typecheck && bun run lint && bun run build

# OpenAPI contract and generated client
bun run api:lint
bun run api:client:check

# Infrastructure, if you changed it
tofu -chdir=infra/gcp fmt -check
tofu -chdir=infra/gcp validate
tofu -chdir=infra/gcp test
```

If you add a migration, run `touch apps/api/src/lib.rs` afterwards. `sqlx::migrate!()`
embeds migrations at compile time, and Cargo will not notice a new file otherwise.

Performance and deployment checks, when relevant:

```sh
infra/perf/measure.sh        # query plans at flash-sale volume
infra/gcp/verify-floci.sh    # emulated-GCP plan and release gates
```

Before pushing, confirm the secret scan is clean:

```sh
gitleaks git --config .gitleaks.toml
```

## What to change, and what not to

**Tests.** Add them where a test pins a behaviour that would otherwise break
silently: concurrency invariants, idempotency, state transitions, and parsing.
Do not add a test that restates an implementation. There is no coverage target,
and adding one would mostly produce tests of getters.

**Secrets.** Never commit a credential, and never add a secret-shaped string to
the `.gitleaks.toml` allowlist to make a scan pass. That allowlist exists for
values that grant no remote access, and each entry explains why. If you need a
new configuration value, add it to `.env.example` with a placeholder and read it
at runtime.

**Dependencies.** Weigh them. A dependency for a problem the standard library or
an existing dependency already solves is not worth the supply-chain surface. If
you do add one, say why in the commit message.

**The payment seam.** `orders.payment_status` exists so a payment adapter can be
added later. Nothing in this repository should write any value other than
`not_required`. Do not implement charges as a side effect of unrelated work.

## Commits and pull requests

Each commit should be reviewable on its own and should explain *why* the change
is being made, not restate the diff. The diff already says what changed.

Use a conventional prefix: `feat`, `fix`, `perf`, `refactor`, `docs`, `test`,
`chore`, `build`, `ci`.

```
fix(checkin): keep a rescan from admitting the same ticket twice

Two gates scanning one credential at the same moment both read status 'valid'
and both wrote 'checked_in', so one guest was admitted twice. Admission is now
a conditional update on the current status, which makes the second write a
no-op and lets the console report "already in" instead of failing.
```

Keep pull requests focused. A pull request that fixes a bug and reformats
unrelated files is two pull requests.

In the description, state what you verified and how. "Tests pass" is weaker than
"the new concurrency test fails on the previous commit and passes on this one".

## Reporting bugs and security issues

Bugs and feature requests: use the issue templates.

Security vulnerabilities: do **not** open a public issue. Follow
[`SECURITY.md`](SECURITY.md).

## Code of conduct

Participation is covered by [`CODE_OF_CONDUCT.md`](CODE_OF_CONDUCT.md).

## Licence

Contributions are accepted under the GNU Affero General Public License v3.0.
See [`LICENSE`](LICENSE). By contributing, you agree your contribution may be
distributed under that licence.
