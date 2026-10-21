# Rotating secrets

How credentials are supplied, how to replace one without downtime, and what each
rotation breaks. The third part is the one people get wrong.

## How secrets are supplied

Two places, and they never overlap.

**Local development** reads `.env`, which is gitignored. The file is parsed
mechanically rather than sourced, because the connection strings contain unquoted
`&` and sourcing would break on them.

**GCP** reads Secret Manager. A Cloud Run revision references each secret by ID
*and by pinned version number*:

```hcl
value_source {
  secret_key_ref {
    secret  = google_secret_manager_secret.runtime_database_url.secret_id
    version = var.runtime_database_secret_version
  }
}
```

The pinned version is deliberate. If revisions followed `latest`, then adding a
secret version would silently redeploy production on the next unrelated apply.
Pinning means a rotation is an explicit act with a reviewable diff, and it means
a rollback is "revert the version variable" rather than "re-upload the old value".

Six secrets exist:

| Secret ID | Consumed by | What it is |
|---|---|---|
| `zeno-prod-runtime-database-url` | api, worker | Pooled URL for the DML-only `zeno_runtime` role |
| `zeno-prod-migration-database-url` | migrate job | Unpooled URL for the schema owner, which is the only credential that may run DDL |
| `zeno-prod-ticket-credential-key` | api | AES-256-GCM key that re-displays issued ticket credentials |
| `zeno-prod-web-auth-cookie-secret` | web | Neon Auth session cookie signing key |
| `zeno-prod-web-storage-access-key` | web | Neon Object Storage access key |
| `zeno-prod-web-storage-secret-access-key` | web | Neon Object Storage secret key |

The API holds no storage credentials, and the web holds no database credentials.
That separation is enforced by which service account can read which secret, not by
convention.

## The rotation procedure

Rotation never edits a secret in place. Secret Manager versions are immutable, so
the sequence is always: add, pin, apply, verify, retire.

1. **Add the new value as a new version.** This does not affect anything running,
   because nothing references the new version number yet.

   ```sh
   printf '%s' "$NEW_VALUE" | gcloud secrets versions add zeno-prod-ticket-credential-key \
     --project=your-gcp-project --data-file=-
   ```

2. **Read back the version number** the add produced. Do not assume it is `2`.

3. **Pin it** in `infra/gcp/terraform.gcp.tfvars`, for example
   `ticket_credential_secret_version = "2"`.

4. **Review the plan.**

   ```sh
   tofu -chdir=infra/gcp plan
   ```

   Expect a new Cloud Run revision. Expect no destruction of the secret resource
   itself. If the plan proposes replacing a service rather than revising it,
   stop: something else changed.

5. **Apply**, then confirm the new revision is serving.

6. **Verify the behaviour** the secret drives, using the per-secret check below.

7. **Disable the retired version.** Do not destroy it immediately.

   ```sh
   gcloud secrets versions disable 1 --secret=zeno-prod-ticket-credential-key --project=your-gcp-project
   ```

   Disabling is reversible and keeps the audit trail. Destruction is for a
   confirmed exposure, not for routine hygiene.

## What each rotation breaks

This is the part worth reading before you touch anything.

### Ticket credential key: rotating breaks re-display, not admission

This one behaves counter-intuitively and the distinction matters.

An issued ticket stores two things derived from the same credential:

- `credential_hash`, a SHA-256 digest, used by the door to find the ticket
- `credential_ciphertext`, an AES-256-GCM sealed copy, used to show the guest their
  QR again

The door looks tickets up **by hash**. Only re-display needs the key. So after
rotating the key:

- Tickets already issued **still check in normally**. Their hashes are unaffected.
- Those same tickets **can no longer be re-displayed**. Decryption fails because
  the ciphertext is sealed under the retired key.
- Newly confirmed orders are unaffected and display correctly.

So do not rotate this key during a live event, and do not assume rotation is
free just because check-in survives. It is safe between events, and it is the
correct response if the key is ever suspected of exposure.

The API **fails closed** without this key. Confirmation and ticket issuance are
disabled rather than degraded, which is the right failure for a component whose
absence would otherwise produce tickets nobody can validate.

### Auth cookie secret: rotating signs everyone out

The web tier refuses to start unless this is at least 32 characters. A new value
invalidates every existing organizer session. There is no graceful migration, so
rotate it deliberately, or during a maintenance window, and expect people to sign
in again.

### Database URL: two secrets, two roles, different privileges

The runtime and migration URLs are separate because they have different
requirements *and* different privileges.

| Secret | Role | Can do |
|---|---|---|
| `zeno-prod-runtime-database-url` | `zeno_runtime` | `SELECT/INSERT/UPDATE/DELETE` on `public`, nothing else |
| `zeno-prod-migration-database-url` | `neondb_owner` | DDL, so it can apply migrations |

The runtime role cannot create, alter, or drop anything. `public` is owned by
`pg_database_owner`, and Postgres 15+ withholds `CREATE` from `PUBLIC` on that
schema, so the restriction is enforced by the database rather than by convention.
This bounds an API compromise to reading and writing application rows: it cannot
drop a table, disable a constraint, or rewrite a trigger, which the owner
credential could.

`ALTER DEFAULT PRIVILEGES` is granted to the runtime role as well, so tables
created by a future migration are readable without a second grant. Without it the
next migration produces a table the runtime cannot read, and the failure shows up
as a 500 in production instead of at migration time.

Rotating the owner password means updating **both** versions and bumping **both**
pinned variables: the runtime and migration URLs are independent credentials now,
but they are still both derived from the owner when it is the owner that rotates.
Rotating only the runtime password is independent and touches only
`zeno-prod-runtime-database-url`.

Order matters if you want zero failed requests: add the new password to the role
first, then update the secret, then apply. Postgres accepts both during the
window. Revoking the old password first causes a hard outage.

#### Creating or recreating the runtime role

```sql
CREATE ROLE zeno_runtime LOGIN PASSWORD '<generated>';
GRANT CONNECT ON DATABASE neondb TO zeno_runtime;
GRANT USAGE ON SCHEMA public TO zeno_runtime;
GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA public TO zeno_runtime;
GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA public TO zeno_runtime;
ALTER DEFAULT PRIVILEGES IN SCHEMA public
  GRANT SELECT, INSERT, UPDATE, DELETE ON TABLES TO zeno_runtime;
ALTER DEFAULT PRIVILEGES IN SCHEMA public
  GRANT USAGE, SELECT ON SEQUENCES TO zeno_runtime;
```

Verify it in **both** directions. A privilege change that is only tested one way
is a hope, not a control:

```sql
-- must succeed
SELECT count(*) FROM public.ticket_types;
-- must fail with "permission denied for schema public"
CREATE TABLE public.probe (id int);
```

The runtime never runs migrations. `sqlx::migrate!()` is reachable only through
the `migrate` command, which the migration job invokes with the owner credential,
so the runtime role never needs DDL and its lack of it is not a latent break.

### Storage keys: rotate at the provider first

These are issued by Neon Object Storage, so the new pair has to exist there before
it exists here. Create the new key pair, pin both secrets, apply, verify a cover
image upload, then revoke the old pair. Revoking first means the web tier cannot
read or write images immediately.

## Verifying a rotation

Verify the behaviour, not the configuration. A secret being present in Secret
Manager proves nothing about whether the running revision can read it.

```sh
# Which revision is serving, and which secret versions it references?
gcloud run services describe zeno-prod-api --region=asia-southeast1 \
  --format='value(status.latestReadyRevisionName)'

# Did a request that depends on the rotated secret actually succeed?
gcloud logging read 'resource.labels.service_name="zeno-prod-api" AND severity>=WARNING' \
  --project=your-gcp-project --freshness=10m --limit=20
```

Then exercise the path the secret protects:

| Rotated secret | Verification that proves it |
|---|---|
| Ticket credential key | Confirm a booking and check the QR renders; then re-load a ticket issued *before* the rotation and confirm it degrades without a 5xx |
| Auth cookie secret | Sign in, then confirm the session persists across a reload |
| Database URL | Run the migrate job and confirm `ok/fail = 1/0` |
| Storage keys | Upload a cover image and confirm it renders |

## If a secret is exposed

Rotation is not the first step. Containment is.

1. **Revoke at the source.** Rotate the credential in the system that issued it
   (Neon, the provider). Everything else is cleanup.
2. **Then** follow the procedure above, out of order if needed: revoke first,
   accept the brief failure, restore. Availability loses to containment.
3. **Confirm it never reached the repository.**

   ```sh
   gitleaks git --config .gitleaks.toml --redact
   ```

4. **Check what the credential could reach** before assuming a limited blast
   radius. A pooled runtime URL cannot run DDL; that is a real boundary. A Neon
   owner URL can.

Deleting a secret version is not remediation on its own. If a value was exposed,
it is exposed regardless of what happens in Secret Manager afterwards.

## Practices that are not negotiable

- Never write a secret value into a file in this repository, including a
  temporary one. The rotation above pipes values through stdin for that reason.
- Never paste a secret into a commit message, an issue, or a screenshot. Gitleaks
  covers the first two; the screenshot habit is a human one.
- Never add a secret-shaped string to `.gitleaks.toml` to make a scan pass. The
  allowlist holds values that grant no remote access, and each entry says why. If
  a real credential lands in the allowlist, the scan stops being evidence.
- Assume anything committed is public, because this repository is. Rewriting
  history does not un-publish a value that has already been fetched.
