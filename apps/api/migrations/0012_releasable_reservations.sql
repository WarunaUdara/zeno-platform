ALTER TABLE reservations
    DROP CONSTRAINT reservations_status_check,
    ADD CONSTRAINT reservations_status_check
        CHECK (status IN ('held', 'expired', 'released'));
