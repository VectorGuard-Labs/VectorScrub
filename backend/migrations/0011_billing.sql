-- Migration 0011: Stripe billing
--
-- One row per Stripe subscription. The row is a snapshot of what Stripe last told us about
-- that subscription; a customer's plan (customers.subscription_tier) is DERIVED from the
-- customer's entitled subscriptions every time one changes (services/billing.rs). Deriving
-- instead of applying "upgrade"/"downgrade" events makes the webhook safe against the things
-- Stripe documents as normal: events delivered twice, out of order, or retried for days.
--
-- Holds no personal data: Stripe identifiers, a status, a price id and dates. Names, card
-- details and billing addresses stay at Stripe and are never stored here.
--
-- customer_id is NULL until a checkout is tied to an account. Stripe does not guarantee that
-- `checkout.session.completed` (which carries our customer reference) arrives before
-- `customer.subscription.created` (which carries the price and status), so the snapshot is
-- kept either way and the two are joined by the subscription id whichever comes first.
--
-- event_created is the Stripe `created` time (unix seconds) of the event applied last. An
-- older event never overwrites a newer snapshot. 0 marks a row created from a checkout
-- before any subscription event has arrived.

CREATE TABLE stripe_subscriptions (
    subscription_id      TEXT PRIMARY KEY,
    stripe_customer_id   TEXT NOT NULL,
    customer_id          UUID REFERENCES customers(id) ON DELETE SET NULL,
    status               TEXT NOT NULL,
    price_id             TEXT,
    cancel_at_period_end BOOLEAN NOT NULL DEFAULT false,
    current_period_end   TIMESTAMPTZ,
    event_created        BIGINT NOT NULL DEFAULT 0,
    created_at           TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at           TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX stripe_subscriptions_customer_idx ON stripe_subscriptions (customer_id);
CREATE INDEX stripe_subscriptions_stripe_customer_idx ON stripe_subscriptions (stripe_customer_id);
