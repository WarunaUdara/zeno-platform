ALTER TABLE events
    ADD COLUMN accessibility_info text,
    ADD COLUMN cancellation_policy text,
    ADD COLUMN support_email text,
    ADD CONSTRAINT events_accessibility_info_length
        CHECK (accessibility_info IS NULL OR char_length(accessibility_info) <= 2000),
    ADD CONSTRAINT events_cancellation_policy_length
        CHECK (cancellation_policy IS NULL OR char_length(cancellation_policy) <= 2000),
    ADD CONSTRAINT events_support_email_format
        CHECK (
            support_email IS NULL
            OR (
                char_length(support_email) BETWEEN 3 AND 254
                AND support_email ~ '^[^[:space:]@]+@[^[:space:]@]+[.][^[:space:]@]+$'
            )
        );
