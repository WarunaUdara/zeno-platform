# 0004. Opaque online QR credentials for the first pilot

- Status: Proposed
- Date: 2026-10-06
- Owners: Security and entry operations

## Context

Entry must reject invalid and previously redeemed tickets. Offline scanning adds stale-revocation and duplicate-use tradeoffs across disconnected devices.

## Decision

The first pilot uses high-entropy opaque credentials in QR codes and online atomic redemption. Credentials reveal no attendee PII. Offline acceptance is deferred unless a pilot requires it and organizers explicitly accept a partitioning/reconciliation design.

## Alternatives considered

- Self-contained signed QR payload: can verify authenticity offline, but cannot provide global single-use or fresh revocation while disconnected.
- Device-specific offline partitions: possible with explicit quantity allocation but adds reconciliation and transfer restrictions.

## Consequences

The scanner needs network connectivity for definitive admission and must show its status clearly. The backend can atomically handle duplicate scans and partial quantity admission.

## Evidence and revisit trigger

Revisit only with venue connectivity measurements and a concrete event workflow requiring offline operation.
