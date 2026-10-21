# 0009. Serverless observability on Cloud Run; evaluate and decline eBPF/Cilium

- Status: Accepted
- Date: 2026-10-15
- Owners: Platform, SRE

## Context

As part of hardening Zeno's observability, performance, and security posture, we evaluated whether to migrate the compute runtime from Google Cloud Run (serverless containers) to Google Kubernetes Engine (GKE) to leverage an eBPF-native platform stack (Cilium CNI, Cilium Hubble, and Tetragon runtime enforcement).

## Decision

**Retain Google Cloud Run as the primary compute platform and implement the four observability pillars using serverless-native technologies. Decline the migration to GKE, Cilium, and eBPF.**

1. **Metrics:** Retain Prometheus `/metrics` scraped by Cloud Monitoring and local Prometheus.
2. **Distributed Tracing:** Export W3C `traceparent`-correlated spans from Axum and Next.js via OpenTelemetry (OTLP) to Google Cloud Trace.
3. **Structured Logging:** Emit structured JSON logs with correlated `trace_id`, `span_id`, and `request_id`, enforcing zero-PII sanitization.
4. **Profiling:** Use in-process user-space profiling (Rust `pprof` crate and Node.js native `--cpu-prof`) to generate CPU and memory flamegraphs without kernel dependencies.
5. **Egress Security:** Use Google Cloud Serverless VPC Access and VPC Service Controls (VPC SC) for perimeter containment and data exfiltration prevention, rather than `CiliumNetworkPolicy`.

### Implementation status

The list above reads as a plan, and a reader cannot tell from it what is running.
Recorded here so the ADR does not become another document that describes an
intention in the present tense.

| Pillar | State |
|---|---|
| 1. Metrics | Implemented. Eleven metrics across four scrape targets, with fourteen alert policies over them. |
| 2. Distributed tracing | Implemented. All three workloads export OTLP spans to Cloud Trace, each carrying the `trace_id` its JSON log reports. The web tier's request span is the root of a browser request and the API's span nests under it. Sampling is parent-based, so the root decides once. Health probes are excluded on purpose. |
| 3. Structured logging | Implemented. JSON logs with `trace_id`, `span_id`, and `request_id`. |
| 4. Profiling | Not implemented. |
| 5. Egress security | Not implemented. No Serverless VPC Access connector and no VPC SC perimeter; the runtime reaches Neon over the public endpoint, authenticated and TLS-terminated but not contained. |

## Alternatives considered and why they were declined

### 1. Migrating to GKE with Cilium CNI and eBPF
- **Incompatible compute boundary:** Cloud Run runs containers under gVisor (user-space kernel sandbox) where raw socket access, `CAP_SYS_ADMIN`, and `CAP_BPF` are unavailable. Adopting eBPF requires provisioning and managing a full Kubernetes cluster.
- **Disproportionate operational and financial cost:** GKE incurs a minimum base cluster management fee plus always-on node pool costs (~$60–$150/month), compared to Cloud Run's $0 idle cost and automatic scale-to-zero. Zeno's event scale (hundreds to thousands of attendees) does not justify managing node pools, control plane upgrades, or CNI lifecycle.
- **Wrong optimization layer:** The measured bottleneck is PostgreSQL. The largest single
  defect found in this repository was a sequential scan inside the expiry worker
  (41 ms over a 151k-row queue, every poll); after bounding it and covering the hot
  paths, the reservation path runs in single-digit milliseconds against Neon in
  Singapore. Every hot-path plan after that change is index-driven, with no
  sequential scans. Optimising kernel packet routing while the database serialises
  writes optimises the layer that is not the constraint.
- **Technology-driven architecture anti-pattern:** Recommending a Kubernetes migration purely to add a named CNI or tracing tool to the stack would optimise for the tool rather than the measured constraint.

## Consequences

- The platform maintains zero idle cost and minimal operational maintenance on Cloud Run.
- Kernel-level network flow mapping (Hubble) and in-kernel syscall termination (Tetragon) are omitted; application security relies on Cloud Armor, IAM least privilege, and VPC Service Controls.
- Profiling is captured via user-space language runtimes rather than continuous kernel-wide sampling.

## Evidence and revisit triggers

Revisit this evaluation only if at least one of the following triggers occurs:
1. Measured sustained throughput exceeds Cloud Run's concurrency ceiling for this
   workload, with instance quota as the limiting factor rather than the database.
2. The system introduces non-HTTP/gRPC protocols (e.g. raw UDP/TCP binary
   streaming) unsupported by Cloud Run.
3. Dedicated bare-metal or multi-cloud node infrastructure becomes an explicit
   business or regulatory requirement.

These thresholds are unmeasured targets, not observations. The concurrency ceiling
must be established by load testing this workload before this ADR is revisited;
none of these conditions has occurred.
