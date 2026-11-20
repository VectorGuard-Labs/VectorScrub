-- Migration 0024: per-customer data keys, wrapped by a key held outside the backend.
--
-- Until now every customer's key was derived from one MASTER_ENCRYPTION_KEY that sat in the
-- backend's environment, so anyone who read that process's environment held every customer's
-- data. Now each customer has their own random data key (DEK). It is stored here only in
-- wrapped form: encrypted by a key-encryption key (KEK) that lives in the key service (PyHSM, or
-- a key derived from the environment in development and tests). See crypto/envelope.rs and
-- services/keys.rs.
--
--   wrapped_dek   the DEK, encrypted by the KEK, bound to this customer. Opaque to the database.
--   kek_provider  which key service wrapped it ('pyhsm' or 'env'). A row wrapped by one provider
--                 cannot be unwrapped by the other; `vectorscrub kms rewrap` moves them.
--   kek_version   which version of the KEK. Rotating the KEK adds a version; rows keep working
--                 until `vectorscrub kms rewrap` re-wraps them under the newest one.
--
-- ON DELETE CASCADE: erasing an account deletes its key too. The customer's encrypted rows are
-- deleted with it, and the key is gone from the live database, so what remains in old backups is
-- unreadable once those backups age out of the key service's own keystore history.

CREATE TABLE customer_keys (
    customer_id  UUID PRIMARY KEY REFERENCES customers(id) ON DELETE CASCADE,
    wrapped_dek  BYTEA NOT NULL,
    kek_provider TEXT NOT NULL CHECK (kek_provider IN ('pyhsm', 'env')),
    kek_version  INTEGER NOT NULL CHECK (kek_version >= 0),
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    rewrapped_at TIMESTAMPTZ
);
CREATE INDEX customer_keys_kek_idx ON customer_keys (kek_provider, kek_version);
