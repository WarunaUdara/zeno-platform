# Architecture decision records

ADRs capture choices that constrain future work. Keep them short and evidence-based; supersede a decision with a new ADR instead of rewriting history.

## Template

```markdown
# NNNN. Decision title

- Status: Proposed | Accepted | Superseded
- Date: YYYY-MM-DD
- Owners: role or names

## Context
What decision is needed and which constraints matter?

## Decision
What will we do?

## Alternatives considered
What credible options were considered and why were they not selected?

## Consequences
What benefits, costs, and follow-up work result?

## Evidence and revisit trigger
What evidence supports this decision, and what observation would cause review?
```

## Initial decisions

- [0001. Modular monolith and context boundaries](0001-modular-monolith.md)
- [0002. Rust API with Axum](0002-rust-axum.md)
- [0003. PostgreSQL is authoritative for inventory](0003-postgres-inventory.md)
- [0004. Opaque online QR credentials for first pilot](0004-online-qr-credentials.md)
- [0005. GCP first, AWS portability exercise later](0005-gcp-first.md)
- [0006. Next.js for event web and organizer console](0006-nextjs-web.md)
- [0007. OpenAPI contract is the cross-client source of truth](0007-openapi-source-of-truth.md)
- [0008. Zeno owns organization membership](0008-application-owned-organizations.md)
- [0009. Serverless observability on Cloud Run; evaluate and decline eBPF/Cilium](0009-serverless-observability-and-ebpf-evaluation.md)

These are proposed decisions until validated by implementation or a pilot. Revisit one only when new measured evidence changes its tradeoff.
