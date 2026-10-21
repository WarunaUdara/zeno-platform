-- The previous request fingerprint included buyer email. Rotate active fingerprints
-- while retaining their keys and responses so retries cannot create duplicate holds.
-- Those keys conflict until their existing replay deadlines expire.
UPDATE idempotency_keys
SET fingerprint = decode(
    md5(gen_random_uuid()::text) || md5(gen_random_uuid()::text),
    'hex'
);

-- Remove purchaser contact data from the schema: this hold flow has no delivery
-- or retrieval use for it. PostgreSQL does not immediately rewrite old tuples;
-- handle table rewrite and backup/WAL expiry through an environment-specific
-- retention procedure if this column ever held real attendee data.
ALTER TABLE reservations DROP COLUMN buyer_email;
