-- Migration 0013: is each broker's public opt-out page still there?
--
-- A nightly job in the agent layer fetches every broker's opt-out page (public pages only, no
-- customer data) and reports what happened. The result is what the dashboard's opt-out guide
-- and the pricing copy show as "last checked", so a link that has gone dead is found by us
-- before a customer finds it.
--
-- What this proves and what it does not: "reachable" means the page loaded for a plain HTTP
-- request. It does NOT mean an automated opt-out succeeds on that site. That needs an actual
-- verified removal, which is tracked separately (removal_requests) and never inferred here.
--
-- One row per source, overwritten by each check. Site names and a status: no personal data.

CREATE TABLE optout_health (
    source               TEXT PRIMARY KEY,
    status               TEXT NOT NULL
                         CHECK (status IN ('reachable', 'captcha', 'blocked', 'not_found', 'unavailable')),
    http_status          INTEGER,
    checked_at           TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    -- The last time the page opened ('reachable' or 'captcha'), kept across later failures so
    -- "it worked until Tuesday" is visible.
    last_reachable_at    TIMESTAMPTZ,
    consecutive_failures INTEGER NOT NULL DEFAULT 0
);
