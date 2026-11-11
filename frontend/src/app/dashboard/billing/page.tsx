"use client";

import { useEffect, useRef, useState } from "react";
import { useRouter } from "next/navigation";
import { Shield, LogOut, CheckCircle, CreditCard } from "lucide-react";
import { getBilling, getProfile, logout } from "@/lib/api";
import { formatDate } from "@/lib/utils";
import type {
  BillingInfo,
  BillingIntervalName,
  BillingPlan,
  CustomerProfile,
} from "@/types";

type PaidTier = "personal" | "premium";

// What each plan costs, for display. The amount actually charged is whatever the Stripe
// price behind the payment link says; these must match it. Annual is ten months' price.
// Only plans the server actually offers are shown (see billing.plans), so a plan listed here
// but not sold (Premium, for now) never appears.
const PRICES: Record<PaidTier, { name: string; month: string; year: string; note?: string; features: string[] }> = {
  personal: {
    name: "Early Access",
    month: "$4.99",
    year: "$49.90",
    note: "Founding members keep this price for as long as they stay subscribed.",
    features: [
      "Checks up to 15 brokers, wherever the site allows it",
      "Unlimited automated opt-out requests for the listings we find",
      "Guided manual opt-outs for the sites we cannot reach",
      "Monthly rescans and reappearance checks",
    ],
  },
  premium: {
    name: "Premium",
    month: "$14.99",
    year: "$149.90",
    features: [
      "Checks up to 19 brokers, wherever the site allows it",
      "Everything in Early Access",
      "Weekly rescans and reappearance checks",
      "Your opt-out requests are processed first",
    ],
  },
};

/** The plan name a customer sees for the tier stored on their account. */
const TIER_LABEL: Record<string, string> = { free: "Free", personal: "Early Access", premium: "Premium" };

const POLL_MS = 3000;
const POLL_MAX = 20; // about a minute

export default function BillingPage() {
  const router = useRouter();
  const [profile, setProfile] = useState<CustomerProfile | null>(null);
  const [billing, setBilling] = useState<BillingInfo | null>(null);
  const [interval, setInterval_] = useState<BillingIntervalName>("month");
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const [activating, setActivating] = useState(false);
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    let cancelled = false;

    // Stripe sends the customer back here after paying. The plan is changed by Stripe's
    // webhook, which can land a few seconds after the redirect, so keep checking briefly.
    const returningFromCheckout =
      new URLSearchParams(window.location.search).get("checkout") === "success";

    async function load(attempt: number) {
      try {
        const [p, b] = await Promise.all([getProfile(), getBilling()]);
        if (cancelled) return;
        setProfile(p);
        setBilling(b);
        setError("");
        const waiting = returningFromCheckout && !b.subscription && attempt < POLL_MAX;
        setActivating(waiting);
        if (waiting) {
          timerRef.current = setTimeout(() => void load(attempt + 1), POLL_MS);
        }
      } catch {
        if (!cancelled) setError("Failed to load billing. Please try again.");
      } finally {
        if (!cancelled) setLoading(false);
      }
    }

    void load(0);
    return () => {
      cancelled = true;
      if (timerRef.current) clearTimeout(timerRef.current);
    };
  }, []);

  function handleSignOut() {
    void logout().finally(() => router.push("/"));
  }

  if (loading) {
    return (
      <div className="min-h-screen flex items-center justify-center">
        <div className="text-gray-400 text-sm animate-pulse">Loading billing…</div>
      </div>
    );
  }

  const sub = billing?.subscription ?? null;
  const currentTier = (profile?.subscription_tier ?? "free").toString().toLowerCase();

  // Offer only what the server sells: the billing periods and plans that have a payment link.
  const offered = billing?.plans ?? [];
  const intervals = (["month", "year"] as const).filter((i) => offered.some((p) => p.interval === i));
  const activeInterval: BillingIntervalName = intervals.includes(interval) ? interval : (intervals[0] ?? "month");
  const tiers = (["personal", "premium"] as const).filter((t) => offered.some((p) => p.tier === t));

  function planFor(tier: PaidTier): BillingPlan | undefined {
    return offered.find((p) => p.tier === tier && p.interval === activeInterval);
  }

  return (
    <div className="min-h-screen">
      <nav className="border-b border-gray-800 px-6 py-4 flex items-center justify-between">
        <div className="flex items-center gap-2">
          <Shield className="text-teal-400 w-5 h-5" aria-hidden="true" />
          <span className="text-white font-bold">VectorScrub</span>
        </div>
        <div className="flex items-center gap-4">
          <button
            onClick={() => router.push("/dashboard")}
            className="text-gray-400 hover:text-white text-sm font-medium transition-colors"
          >
            Dashboard
          </button>
          <button
            onClick={handleSignOut}
            className="flex items-center gap-1.5 text-gray-400 hover:text-white text-sm transition-colors"
            aria-label="Sign out"
          >
            <LogOut className="w-4 h-4" />
            Sign out
          </button>
        </div>
      </nav>

      <main className="max-w-4xl mx-auto px-6 py-10">
        <h1 className="text-2xl font-bold text-white mb-2">Billing</h1>
        <p className="text-gray-400 text-sm mb-8">Your plan and subscription</p>

        {error && (
          <div role="alert" className="bg-red-950 border border-red-800 text-red-300 rounded-lg px-4 py-3 mb-6 text-sm">
            {error}
          </div>
        )}

        {activating && (
          <div role="status" className="bg-teal-950 border border-teal-800 text-teal-300 rounded-lg px-4 py-3 mb-6 text-sm">
            Thank you. Your payment went through and your plan is being activated.
          </div>
        )}

        {/* Current plan */}
        <section className="bg-gray-900 border border-gray-800 rounded-xl p-6 mb-10" aria-labelledby="current-plan">
          <div className="flex items-start justify-between gap-4">
            <div>
              <h2 id="current-plan" className="text-gray-400 text-sm mb-1">Current plan</h2>
              <div className="text-white text-xl font-semibold">{TIER_LABEL[currentTier] ?? currentTier}</div>
              {sub ? (
                <p className="text-gray-400 text-sm mt-2">
                  {PRICES[sub.tier].name}, billed {sub.interval === "year" ? "yearly" : "monthly"}.{" "}
                  {sub.cancel_at_period_end ? (
                    <>
                      Cancelled. You keep {PRICES[sub.tier].name} until{" "}
                      <span className="text-white">{formatDate(sub.current_period_end)}</span>, then your account
                      moves to Free.
                    </>
                  ) : (
                    <>
                      Renews on <span className="text-white">{formatDate(sub.current_period_end)}</span>.
                    </>
                  )}
                  {sub.status === "past_due" && (
                    <span className="block text-orange-400 mt-1">
                      Your last payment failed. Update your card to keep your plan.
                    </span>
                  )}
                </p>
              ) : (
                <p className="text-gray-400 text-sm mt-2">
                  {currentTier === "free"
                    ? "Free accounts get one scan every 7 days, up to 3 automated opt-out requests, and the manual opt-out guide."
                    : "This plan was set on your account directly, not through a subscription."}
                </p>
              )}
            </div>
            {billing?.portal_url && sub && (
              <a
                href={billing.portal_url}
                rel="noopener noreferrer"
                className="flex items-center gap-2 bg-gray-800 hover:bg-gray-700 text-white px-4 py-2 rounded-lg text-sm font-medium transition-colors whitespace-nowrap"
              >
                <CreditCard className="w-4 h-4" aria-hidden="true" />
                Manage subscription
              </a>
            )}
          </div>
        </section>

        {/* Plans */}
        {!billing?.enabled || (tiers.length === 0 && !sub) ? (
          <p className="text-gray-500 text-sm">Online purchase is not available yet.</p>
        ) : tiers.length === 0 ? null : (
          <section aria-labelledby="plans">
            <div className="flex items-center justify-between mb-6">
              <h2 id="plans" className="text-lg font-semibold text-white">
                {sub ? "Plans" : "Choose a plan"}
              </h2>
              {intervals.length > 1 && (
              <div role="group" aria-label="Billing period" className="inline-flex rounded-lg border border-gray-700 p-0.5">
                {(["month", "year"] as const).map((i) => (
                  <button
                    key={i}
                    onClick={() => setInterval_(i)}
                    aria-pressed={activeInterval === i}
                    className={`px-4 py-1.5 rounded-md text-sm font-medium transition-colors ${
                      activeInterval === i ? "bg-teal-600 text-white" : "text-gray-400 hover:text-white"
                    }`}
                  >
                    {i === "month" ? "Monthly" : "Annual"}
                  </button>
                ))}
              </div>
              )}
            </div>
            {activeInterval === "year" && (
              <p className="text-teal-400 text-sm mb-4">Annual billing: pay for 10 months, get 12.</p>
            )}

            <div className={`grid grid-cols-1 gap-6 ${tiers.length > 1 ? "sm:grid-cols-2" : "max-w-md"}`}>
              {tiers.map((tier) => {
                const info = PRICES[tier];
                const plan = planFor(tier);
                const isCurrent = sub?.tier === tier && sub.interval === activeInterval;
                return (
                  <div key={tier} className="bg-gray-900 border border-gray-800 rounded-xl p-6 flex flex-col">
                    <h3 className="text-white font-semibold text-lg">{info.name}</h3>
                    <div className="text-white text-3xl font-bold mt-2">
                      {activeInterval === "year" ? info.year : info.month}
                      <span className="text-gray-400 text-sm font-normal">
                        {activeInterval === "year" ? " / year" : " / month"}
                      </span>
                    </div>
                    {info.note && <p className="text-teal-300/80 text-xs mt-2">{info.note}</p>}
                    <ul className="mt-4 mb-6 space-y-2 flex-1">
                      {info.features.map((f) => (
                        <li key={f} className="flex items-start gap-2 text-gray-300 text-sm">
                          <CheckCircle className="w-4 h-4 text-teal-400 mt-0.5 flex-shrink-0" aria-hidden="true" />
                          {f}
                        </li>
                      ))}
                    </ul>
                    {isCurrent ? (
                      <div className="text-center text-teal-400 text-sm font-medium py-2.5">Your current plan</div>
                    ) : sub ? (
                      <div className="text-center text-gray-500 text-sm py-2.5">
                        Use Manage subscription to switch plans
                      </div>
                    ) : plan ? (
                      <a
                        href={plan.checkout_url}
                        rel="noopener noreferrer"
                        className="block text-center bg-teal-600 hover:bg-teal-500 text-white px-5 py-2.5 rounded-lg text-sm font-medium transition-colors"
                      >
                        Subscribe to {info.name}
                      </a>
                    ) : (
                      <div className="text-center text-gray-500 text-sm py-2.5">Not available yet</div>
                    )}
                  </div>
                );
              })}
            </div>
          </section>
        )}
      </main>
    </div>
  );
}
