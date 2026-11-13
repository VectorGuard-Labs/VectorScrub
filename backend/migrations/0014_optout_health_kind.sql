-- Migration 0014: opt-out page health now covers two kinds of page
--
-- 'optout'    : the opt-out pages of the brokers VectorScrub itself handles (migration 0013).
--               `source` is a site name such as 'spokeo.com'.
-- 'ca_rights' : the page each company in the California Data Broker Registry names for privacy
--               requests. `source` is the registry's Broker ID (digits). Several companies can
--               share a site, and a registry company's page can be on the same site as one of
--               ours, so the two kinds must not share a key.
--
-- Existing rows are all 'optout'. Nothing else changes: still public pages only, no personal data.

ALTER TABLE optout_health
    ADD COLUMN kind TEXT NOT NULL DEFAULT 'optout' CHECK (kind IN ('optout', 'ca_rights'));

ALTER TABLE optout_health DROP CONSTRAINT optout_health_pkey;
ALTER TABLE optout_health ADD PRIMARY KEY (kind, source);
