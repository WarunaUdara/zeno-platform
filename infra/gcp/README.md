# GCP deployment

This OpenTofu root provisions the GCP foundation and, in a later apply, the Zeno Cloud Run workloads. It defaults to **no Cloud Run workloads and no public web access**. State is stored in a GCS bucket, and secret values are added directly to Secret Manager outside OpenTofu state.

The target layout and release gates are described in [the deployment architecture](../../docs/architecture/GCP_DEPLOYMENT.md).

## Prerequisites

- OpenTofu 1.9 or newer, `gcloud`, Docker Buildx, and an authenticated GCP user with permission to enable APIs, create the state bucket, Artifact Registry, service accounts, secrets, and Cloud Run resources in the target project.
- An isolated Neon deployment branch with a **pooled least-privilege runtime URL** and a separate **unpooled migration URL**. Do not use the linked `production` branch or its owner URL for this demo deployment.
- The Neon Auth branch must list the origin that serves the sign-in form as a trusted domain. Neon Auth validates the `Origin` header, so an origin that is missing from that list makes every `POST /api/auth/sign-up/email` fail with `403 {"message":"Invalid origin","code":"INVALID_ORIGIN"}`. The failure is easy to miss: the catalog, event pages, API health, and the migrate job all stay green, and the sign-up form simply redraws with no error shown. Because the `run.app` URL is not known until the workloads are applied, add the domain after the first workload apply, using the `web_url` output:

```sh
neon neon-auth domain list
neon neon-auth domain add "$(tofu -chdir=infra/gcp output -raw web_url)"
```

When you publish a custom domain later, add that origin as well; the `run.app` origin and the custom domain are separate entries.
- Billing enabled in the GCP project. Reservation expiry uses a bounded Cloud Run Job invoked by Cloud Scheduler. Public publishing additionally needs a DNS name you control; it creates a global external HTTPS load balancer and Cloud Armor edge policy.

Reservation expiry runs as a standard Cloud Run Job. Cloud Scheduler invokes the Cloud Run Jobs API every minute by default; each run expires up to 100 holds, deletes up to 1,000 idempotency records past their 24-hour replay window, records its outcome, and exits. Its dedicated scheduler service account can invoke only this job and has no database-secret access. The worker job uses the runtime database credential and updates a stable heartbeat. A one-minute interval permits almost a minute of delay after a hold expires, so tune `expiry_schedule` only after setting an acceptable hold-release objective. Database locks make overlapping batches safe; check backlog recovery and failure alerts before relying on this for a pilot. The continuous local worker runs idempotency cleanup hourly by default to avoid adding a cleanup query to every poll.

The suggested GCP location is `asia-southeast1`, close to the linked Neon project's current Singapore region. Confirm your intended attendee location and data-residency needs before applying.

## Bootstrap state and foundation

Choose a globally unique GCS bucket name owned by the target project. This bootstrap bucket is created outside OpenTofu because the GCS backend must exist before `tofu init`.

```sh
gcloud auth list
gcloud config get-value project
gcloud storage buckets create gs://YOUR_UNIQUE_STATE_BUCKET \
  --project=YOUR_GCP_PROJECT \
  --location=asia-southeast1 \
  --uniform-bucket-level-access \
  --public-access-prevention
gcloud storage buckets update gs://YOUR_UNIQUE_STATE_BUCKET --versioning
cp infra/gcp/terraform.tfvars.example infra/gcp/terraform.tfvars
```

Edit the ignored `infra/gcp/terraform.tfvars` with the confirmed project id. Keep the initial values `deploy_workloads = false` and `publish_web = false`. Initialize and apply only the foundation:

```sh
tofu -chdir=infra/gcp init \
  -backend-config="bucket=YOUR_UNIQUE_STATE_BUCKET" \
  -backend-config="prefix=zeno/demo"
tofu -chdir=infra/gcp plan
tofu -chdir=infra/gcp apply
```

Review the plan. The foundation enables the required APIs and creates an Artifact Registry repository with immutable image tags, service accounts, Secret Manager secret metadata, and narrow secret-access grants. Terraform never receives database URL values.

When `deploy_workloads = true`, Cloud Monitoring policies alert on a sustained 5xx ratio above 5% and successful-request p95 latency above 2.5 seconds for both Cloud Run services, and on failed expiry-job executions. The service thresholds are starters, not production SLOs. Set `monitoring_notification_channels` to fully qualified names of pre-created, verified channels (for example `projects/PROJECT_ID/notificationChannels/CHANNEL_ID`) to deliver incidents to responders. This input is attached to every policy, and `production_release = true` is rejected when it is empty. Keep recipient setup and channel verification in the owning GCP project; do not put personal destinations in the checked-in example file.

## Add secrets and build images

After the foundation apply, add the isolated branch's runtime and migration URLs as Secret Manager version 1 using protected local files or a secure stdin workflow. Use the exact IDs printed by the Terraform outputs. Do not put either URL in `.tfvars`, Terraform variables, build arguments, shell command arguments, GitHub variables, logs, or source control. Never copy `NEON_DB_OWNER_CONNECTION_STRING` into a runtime secret.

For organizer sign-in, configure `neon_auth_base_url` and `neon_auth_jwks_url` from an isolated Neon Auth project. Add a unique random cookie-signing secret of at least 32 characters to the Terraform-created `web_auth_cookie_secret_id` using Secret Manager, then set `web_auth_cookie_secret_version` to its pinned numeric version. Do not add the secret value to Terraform state or a command-line argument. The web runtime alone receives access to that secret; the API receives only the issuer and JWKS URLs. Do not point this setup at the linked production branch.

For event cover images, create Secret Manager versions for the Neon branch's `AWS_ACCESS_KEY_ID` and `AWS_SECRET_ACCESS_KEY` using the Terraform outputs `web_storage_access_key_secret_id` and `web_storage_secret_access_key_secret_id`. Set `web_storage_access_key_secret_version`, `web_storage_secret_access_key_secret_version`, `neon_object_storage_endpoint_url`, and `neon_object_storage_region` in the ignored tfvars file. Keep credential values out of Terraform inputs/state, command-line arguments, logs, and source control. Only the web runtime receives these credentials; the API and reservation worker do not. Use credentials and an endpoint from the same Neon branch as the application's database. `production_release = true` requires pinned storage secret versions and a supported region.

Configure Docker authentication for the selected Artifact Registry location and build for Cloud Run's Linux AMD64 runtime. Use a unique commit tag, then record the resulting image digests for `api_image` and `web_image` in the ignored tfvars file:

```sh
gcloud auth configure-docker asia-southeast1-docker.pkg.dev
docker buildx build --platform linux/amd64 --push \
  --tag asia-southeast1-docker.pkg.dev/YOUR_GCP_PROJECT/zeno-demo-images/api:YOUR_COMMIT \
  --file apps/api/Dockerfile .
docker buildx build --platform linux/amd64 --push \
  --tag asia-southeast1-docker.pkg.dev/YOUR_GCP_PROJECT/zeno-demo-images/web:YOUR_COMMIT \
  --file apps/web/Dockerfile .
```

Use `.../api@sha256:...` and `.../web@sha256:...` digest references in Terraform so a deployment always identifies the exact image content. Then set `deploy_workloads = true`, keep `publish_web = false`, and apply. This creates authenticated, non-public web/API services, the scheduled expiry job, its one-minute Cloud Scheduler trigger, and a one-off migration job. You can disable the scheduler trigger for a private smoke deployment by setting `expiry_schedule_enabled = false`; the Cloud Run Job remains available for manual execution.

## Release gate

Run the migration job only after verifying the isolated Neon target and reviewing the migration diff:

```sh
gcloud run jobs execute zeno-demo-migrate \
  --project=YOUR_GCP_PROJECT \
  --region=asia-southeast1 \
  --wait
```

Inspect the job execution result and logs. Test the still-private web service through the authenticated Cloud Run proxy:

```sh
gcloud run services proxy zeno-demo-web \
  --project=YOUR_GCP_PROJECT \
  --region=asia-southeast1 \
  --port=3012
```

In another terminal, run the event-page and synthetic idempotent-reservation smoke test through `http://127.0.0.1:3012`. The proxy authenticates as the active gcloud user; the web service then calls the IAM-protected API using its runtime identity. For a public synthetic preview, set `public_domain` to a DNS name you control, point its A record to the `public_web_ip` output, leave `public_reservation_rate_limit_preview = true`, and set `publish_web = true`. Review Cloud Armor logs and test expected traffic from shared networks; tune the per-IP limit from that evidence. Preview mode is public and does not enforce the throttle, so do not invite real attendees yet. Before asserting a production release, configure Auth with an isolated Neon project, verify the browser sign-in/session lifecycle and tenant denial, create and verify a responder notification channel, and add its full resource name to `monitoring_notification_channels`. Then set preview to false and `production_release = true`; the plan is rejected unless public workloads, HTTPS issuer/JWKS URLs, a pinned cookie-secret version, scheduled expiry, the custom domain, the enforced rate rule, and notification delivery are configured. Review the load balancer, Cloud Armor policy, public web invocation grant, and ingress restriction before applying. HTTPS becomes ready after DNS points to the reserved IP and Google provisions the managed certificate. Direct public access to the Cloud Run web URL is blocked in this mode. The API continues to have only the web runtime service account as an invoker.

`production_release` is an infrastructure configuration gate, not an authorization to launch ticket sales. The current product creates temporary anonymous holds and issues entry tickets on confirmation, but takes no payment: every order is written with `payment_status = 'not_required'`, which is the seam a provider integration would replace. Provider-backed payment, Auth behavior, recovery, support ownership, privacy review, and the other launch gates remain separate evidence requirements.

To run a single expiry batch manually and inspect recent executions:

```sh
gcloud run jobs execute zeno-demo-expire-holds --project=YOUR_GCP_PROJECT --region=asia-southeast1 --wait
gcloud run jobs executions list --job=zeno-demo-expire-holds --project=YOUR_GCP_PROJECT --region=asia-southeast1
```

The first deployment is an anonymous synthetic demo, not live ticket sales. Cloud Armor's per-IP throttle is approximate abuse mitigation, can affect people behind shared networks, and is not an identity-based quota or a defense against distributed bot traffic. Before inviting real attendees, add account/tenant authorization, measure and tune abuse controls, configure a verified alert notification channel, rehearse backup restoration, review retention, and test schema rollout/rollback.

## Teardown

`tofu destroy` is intentionally blocked for the three Secret Manager resources and Cloud Run workloads by deletion protection. To tear down a disposable demo, first set `publish_web = false`, `expiry_schedule_enabled = false`, then remove the relevant `prevent_destroy`/deletion protections in a reviewed local change and plan the destroy. Preserve the state bucket and secret data until the teardown and data-retention decision are complete. Do not destroy a shared GCP project or the linked Neon production branch as part of this process.
