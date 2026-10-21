# Cloud portability and sovereignty

## A realistic portability goal

Design around portable application and data contracts, then implement GCP as the first adapter. AWS is a later deployment target proved by a deploy-and-restore exercise. Avoid claiming zero vendor lock-in: managed identity, networking, encrypted storage, managed databases, and operational controls always have provider-specific behavior.

The baseline should use OCI containers, Kubernetes for the application runtime, PostgreSQL-compatible SQL, OpenID Connect, OpenTelemetry OTLP, standard HTTP/REST, OpenAPI, SMTP-compatible email/provider ports, and adapters for blobs, queues, payments, identity, secrets, and KMS. Keep provider API types out of domain and application modules.

Do not build an abstraction for every service in anticipation. A port exists where product behavior requires more than one implementation or where there is a clear portable contract. Keep the adapter honest about capability differences and failure semantics.

## Provider map

| Capability | GCP first implementation | AWS comparison | Portable boundary / caveat |
| --- | --- | --- | --- |
| Workload runtime | Cloud Run | AWS App Runner or ECS Fargate | OCI image and HTTP process contract; ingress, identity, scaling, and job semantics differ |
| PostgreSQL | Neon for the demo; Cloud SQL if managed GCP ownership is needed | Neon or RDS for PostgreSQL/Aurora PostgreSQL | PostgreSQL SQL/migrations; backup, extensions, HA, failover, pooling, and parameter support differ |
| Object storage | Cloud Storage | S3 | Small application `ObjectStore` port and checksum/retention semantics; presigned URL and IAM behavior need adapters |
| Messaging (later) | Pub/Sub | SQS/SNS or EventBridge | At-least-once message port with explicit ack/retry/dead-letter; ordering and retention differ |
| Secret/key service | Secret Manager / Cloud KMS | Secrets Manager / KMS | Envelope-encryption/secret retrieval ports, key rotation and IAM policies remain provider-specific |
| Runtime workload identity | Cloud Run service identity | ECS task role or App Runner instance role | Short-lived workload identity and scoped policy; token delivery and trust setup differ |
| Build/deploy identity | GitHub Actions OIDC federation to GCP | GitHub Actions OIDC federation to AWS | GitHub OIDC trust pinned to repository/environment; provider roles are different |
| Telemetry | OTLP to Collector and selected backend | Same OTLP agent/collector | Instrumentation and semantic conventions portable; provider monitoring dashboards/exporters not identical |
| Email and payments | Organization-selected provider | Organization-selected provider | Application adapter, signed webhook and idempotency contract; settlement/legal support is geographic |
| DNS/TLS/edge | GCP-compatible ingress and chosen CDN/DNS | AWS-compatible ingress and chosen CDN/DNS | DNS, TLS, forwarded identity, WAF and cache behavior require configuration-specific checks |

Neon, GCP Cloud SQL, and AWS-managed PostgreSQL are not interchangeable at the click of a switch. Portability means application SQL and data can be restored from verified backups with supported schema/extension parity, and the application can run against a tested adapter. Do not translate live databases between providers without a planned migration and rollback window.

## First cloud deployment

The first deployment is Cloud Run with Neon: it keeps the system small while exercising private service-to-service identity, autoscaling limits, secret access, migrations, and observability. Measure the application before adding another runtime or database. A Kubernetes deployment is a separate exercise, justified only when Kubernetes operations are an actual requirement; if you run it, compare startup and request latency, scale behavior, recovery, connection pressure, operational effort, and measured cost. Keep clusters, replica counts, node pools, retention, and idle time bounded, and document a verified teardown path.

Terraform contains common inputs/outputs plus separate GCP and AWS environment roots. Do not create a fake universal resource module with provider magic. Run formatting/validation, review full plans, separate environments/projects/accounts, protect remote state, version modules, and enforce destroy safeguards for production data.

Choose region only after identifying the pilot's organizer/attendee geography, payment provider, legal/retention requirements, latency, and cost. A nearby region is not proof of sovereignty or compliance. Document where primary data, backups, email, logs, support access, and encryption keys reside and which operators can access them.

## Sovereignty modes

Define sovereignty by control of data, jurisdiction, operator, keys, and exit path—not by a marketing label. A managed GCP deployment can provide configuration choices but still relies on Google and its operations. A high-control deployment may require customer-chosen region, customer-managed keys, self-hosted runtime/database/storage, restricted support access, and reproducible export/restore artifacts. State residual provider access and legal exposure explicitly.

For an AWS move, provision an isolated AWS test environment, deploy the same immutable application artifacts, migrate a scrubbed GCP backup through PostgreSQL tools, copy object checksums, replay a known event sequence, compare record counts and behavior, and measure downtime and cost. Use no production attendee data for the first portability proof.

## Portability acceptance evidence

- Domain and application crates have no imports of Google or AWS SDK types.
- Each provider adapter implements a documented error/retry/consistency contract.
- A synthetic tenant's events, orders, ticket assignments, check-ins, audit history, and object metadata export without provider-native identifiers being the only references.
- A GCP-origin backup restores to tested AWS PostgreSQL compatibility (or a documented incompatibility blocks the move).
- Object export includes checksum and metadata integrity verification.
- Workload CI identity and runtime identity are distinct and use short-lived credentials.
- The alternate-provider exercise compares functional behavior, recovery time, cost, operational staffing, and unsupported managed features.
