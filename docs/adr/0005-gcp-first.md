# 0005. GCP first, AWS portability exercise later

- Status: Proposed
- Date: 2026-10-06
- Owners: Platform

## Context

The project is named for the initial GCP focus and needs practical cloud and SRE evidence. Operating two providers during an early solo build would dilute reliability work.

## Decision

Deliver and measure one GCP deployment first. Keep application-facing interfaces portable where they reduce real coupling, then document and execute an AWS deployment exercise as a separate proof. Do not claim production multi-cloud readiness from Terraform modules or abstraction interfaces alone.

## Alternatives considered

- Simultaneous GCP and AWS production: expensive operational surface with no customer need.
- GCP-only provider-specific application: fastest, but can make a later migration exercise unnecessarily expensive.

## Consequences

Some managed service behavior and observability remain provider-specific. Portability is assessed through a deployment and data/recovery exercise, with explicit gaps and costs.

## Evidence and revisit trigger

Revisit if a customer or regulatory requirement needs another region/provider or if GCP constraints are material to service objectives.
