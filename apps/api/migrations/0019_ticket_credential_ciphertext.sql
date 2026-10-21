-- Allow a ticket's QR code to be shown again to its owner.
--
-- credential_hash alone is sufficient to admit a guest at the door but makes the
-- code unrecoverable, so an attendee who closes the tab loses their ticket. This
-- stores the same credential sealed with AES-256-GCM under a key held outside the
-- database, so it can be re-rendered for someone who proves they own the order.
--
-- A leaked database therefore yields a digest that admits nothing, and the
-- ciphertext is useless without the key from ZENO_TICKET_CREDENTIAL_KEY, which is
-- not stored in the database or in the same backup.

ALTER TABLE tickets
    ADD COLUMN credential_ciphertext bytea
        CHECK (credential_ciphertext IS NULL OR octet_length(credential_ciphertext) BETWEEN 40 AND 256),
    ADD CONSTRAINT tickets_credential_columns_together
        CHECK ((credential_hash IS NULL) = (credential_ciphertext IS NULL));