-- Migration 0019: what a pilot needs to learn from its first customers.
--
-- pilot_cohort: a neutral group label ('a' | 'b' | 'c') so results can be compared across
--   kinds of participant. What each letter MEANS is kept by the operator outside the database
--   on purpose: a label such as "at risk" is itself sensitive and does not belong next to an
--   account.
-- operator_time: minutes a person spent on a customer (no free text, so nothing identifying),
--   so the pilot can say what a customer costs to serve. SET NULL on erasure keeps the economics
--   without keeping the person.

ALTER TABLE customers
    ADD COLUMN pilot_cohort TEXT CHECK (pilot_cohort IN ('a', 'b', 'c'));

CREATE TABLE operator_time (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    customer_id UUID REFERENCES customers(id) ON DELETE SET NULL,
    kind        TEXT NOT NULL CHECK (kind IN ('intake', 'review', 'submission', 'followup', 'verification', 'report', 'other')),
    minutes     INT NOT NULL CHECK (minutes BETWEEN 1 AND 600),
    logged_at   TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX operator_time_customer_idx ON operator_time (customer_id);
