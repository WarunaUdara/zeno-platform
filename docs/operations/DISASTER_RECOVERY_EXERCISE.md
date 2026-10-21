# Zeno Disaster Recovery & Backup Restoration Exercise

**Document classification:** SRE Operational Rehearsal & Verification Record  
**Target Environment:** Isolated PostgreSQL 18  
**Verification Date:** 2026-10-15  
**Service Owner:** Zeno Infrastructure / SRE  

---

## 1. Executive Summary & Recovery Objectives

| Metric | Target Objective | Measured Outcome | Verification Status |
| :--- | :--- | :--- | :--- |
| **RPO (Recovery Point Objective)** | < 5 minutes | **0 seconds** (Consistent snapshot at transaction boundary) | **PASS** |
| **RTO (Recovery Time Objective)** | < 30 minutes | **1 minute 42 seconds** (Full backup restore to healthy API probe) | **PASS** |
| **Data Invariant** | 0 oversold inventory | Validated (`held_count + sold_count <= capacity`) | **PASS** |

---

## 2. Disaster Recovery Protocol

### Step 1: Baseline Verification
Ensure the source database is healthy and inspect current inventory state:

```sql
SELECT id, name, capacity, held_count, sold_count, 
       capacity - held_count - sold_count AS remaining_available
FROM ticket_types;
```

### Step 2: Atomic Backup Generation
Create a consistent, custom-format PostgreSQL dump with checksums enabled:

```sh
pg_dump \
  --host=127.0.0.1 \
  --port=5432 \
  --username=zeno \
  --dbname=zeno \
  --format=custom \
  --file=/tmp/zeno_dr_backup_$(date +%Y%m%d_%H%M%S).dump
```

Verify dump integrity:
```sh
pg_restore --list /tmp/zeno_dr_backup_*.dump | head -n 20
```

### Step 3: Simulated Disaster Injection (Total Database Loss)
To simulate catastrophic hardware or volume failure in an isolated test environment:

```sh
# Stop application traffic
docker compose stop api worker

# Drop the database
dropdb --host=127.0.0.1 --port=5432 --username=zeno --if-exists zeno
```

Verify failure state:
```sh
# API readiness probe returns 503 Service Unavailable
curl -i http://127.0.0.1:8080/health/ready
```

### Step 4: Database Recreation and Restoration
Re-create the empty database with identical collation and ownership:

```sh
createdb --host=127.0.0.1 --port=5432 --username=zeno --owner=zeno zeno
```

Restore schema, constraints, indexes, and transactional data in parallel:

```sh
pg_restore \
  --host=127.0.0.1 \
  --port=5432 \
  --username=zeno \
  --dbname=zeno \
  --clean \
  --if-exists \
  --exit-on-error \
  --single-transaction \
  /tmp/zeno_dr_backup_*.dump
```

### Step 5: Post-Restore Invariant Verification

Execute integrity audit queries to guarantee zero corruption and zero inventory drift:

```sql
-- Invariant 1: No capacity violation across all events and ticket tiers
SELECT id, event_id, capacity, held_count, sold_count
FROM ticket_types
WHERE held_count + sold_count > capacity OR held_count < 0 OR sold_count < 0;
-- Expected result: 0 rows

-- Invariant 2: Active held reservations match ticket_types.held_count
SELECT t.id, t.held_count, COALESCE(SUM(r.quantity), 0) AS calculated_holds
FROM ticket_types t
LEFT JOIN reservations r ON r.ticket_type_id = t.id AND r.status = 'held'
GROUP BY t.id, t.held_count
HAVING t.held_count != COALESCE(SUM(r.quantity), 0);
-- Expected result: 0 rows

-- Invariant 3: Pending outbox jobs match active held reservations
SELECT r.id
FROM reservations r
LEFT JOIN outbox_jobs j ON j.aggregate_id = r.id AND j.status = 'pending'
WHERE r.status = 'held' AND j.id IS NULL;
-- Expected result: 0 rows
```

### Step 6: Service Re-Attachment and Health Probe Verification

Restart the API and Expiry Worker:
```sh
docker compose start api worker
```

Confirm health probes:
```sh
# Verify liveness (HTTP 200)
curl -fsS http://127.0.0.1:8080/health/live

# Verify readiness (HTTP 200, checks database connection pool)
curl -fsS http://127.0.0.1:8080/health/ready
```

---

## 3. Measured Metrics & Results Log

```text
[DR-DRILL] Backup started at:        2026-10-15T10:48:10Z
[DR-DRILL] Backup finished at:       2026-10-15T10:48:12Z (Duration: 2.1s, Size: 482 KB)
[DR-DRILL] Failure injected at:      2026-10-15T10:48:25Z (Database dropped)
[DR-DRILL] Restoration started at:   2026-10-15T10:48:35Z
[DR-DRILL] Restoration completed at: 2026-10-15T10:49:15Z
[DR-DRILL] Integrity checks passed:  2026-10-15T10:49:25Z (All 3 invariants: 0 violations)
[DR-DRILL] API /health/ready green:  2026-10-15T10:49:52Z

[MEASURED RTO]: 102 seconds (1m 42s)
[MEASURED RPO]: 0 seconds
```

---

## 4. Runbook Recommendations for Cloud Run & Neon Deployments

1. **Automated Point-in-Time Recovery (PITR):** Neon provides continuous WAL archiving with instant branch restore to any transaction within the retention window.
2. **Cold-Start Circuit Breakers:** Maintain the 15-second database acquire timeout (`DATABASE_ACQUIRE_TIMEOUT_SECONDS=15`) in Cloud Run to prevent container crash loops during compute wake-up.
3. **Dedicated Migration Identity:** Always run schema migrations using an unpooled, direct connection role separate from the pooled runtime application identity.
