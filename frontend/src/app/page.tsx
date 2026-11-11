import Link from "next/link";
import { Shield, Search, Trash2, Bell, BarChart3, Lock } from "lucide-react";
import { fetchPublicCoverage, planCoverage, type PlanTier } from "@/lib/coverage";

const features = [
  {
    icon: Search,
    title: "AI-Powered Discovery",
    description:
      "Our agents scan people-search engines, data brokers, and public directories to find where your information is exposed.",
  },
  {
    icon: Shield,
    title: "PII Detection",
    description:
      "AI classifies exactly what's exposed — name, address, phone, email, family members — and scores each exposure by severity.",
  },
  {
    icon: Trash2,
    title: "Automated Removal",
    description:
      "We identify the correct opt-out procedure for each source, prepare authorized removal requests, and track the results.",
  },
  {
    icon: Bell,
    title: "Continuous Monitoring",
    description:
      "We keep watching. When removed information reappears, we alert you and act again.",
  },
  {
    icon: BarChart3,
    title: "Privacy Score",
    description:
      "A live score shows your exposure trend over time — before and after removals.",
  },
  {
    icon: Lock,
    title: "Security-First",
    description:
      "Your identity data is encrypted with AES-256-GCM. It is never sold, never used to train models, and never shared.",
  },
];

const plans: {
  name: string;
  tier: PlanTier;
  price: string;
  per?: string;
  description: string;
  badge: string | null;
  note: string | null;
  features: string[];
  cta: string;
  highlight: boolean;
}[] = [
  {
    name: "Free",
    tier: "free",
    price: "$0",
    description: "See where you are listed",
    badge: null,
    note: null,
    features: [
      "Checks up to 4 brokers, once every 7 days",
      "Up to 3 automated opt-out requests",
      "Step-by-step guide for opting out by hand",
      "One identity",
    ],
    cta: "Get Started",
    highlight: false,
  },
  {
    name: "Early Access",
    tier: "personal",
    price: "$4.99",
    per: "/month",
    description: "Opt out of the listings we find",
    badge: "Early access",
    note: "Founding members keep this price for as long as they stay subscribed, even when it goes up for new members. Cancel any time.",
    features: [
      "Checks up to 15 brokers, wherever the site allows it",
      "Unlimited automated opt-out requests for the listings we find",
      "Guided manual opt-outs for the sites we cannot reach",
      "Monthly rescans and reappearance checks",
      "Alerts when removed information reappears",
      "One identity",
    ],
    cta: "Start Early Access",
    highlight: true,
  },
];

export default async function LandingPage() {
  const coverage = await fetchPublicCoverage();
  const overall = planCoverage("premium", coverage);
  return (
    <main className="flex flex-col min-h-screen">
      {/* Nav */}
      <nav className="border-b border-gray-800 px-6 py-4 flex items-center justify-between max-w-7xl mx-auto w-full">
        <div className="flex items-center gap-2">
          <Shield className="text-teal-400 w-6 h-6" aria-hidden="true" />
          <span className="font-bold text-white text-lg tracking-tight">
            VectorScrub
          </span>
          <span className="text-xs text-gray-500 ml-1">by VectorGuard Labs</span>
        </div>
        <div className="flex items-center gap-4">
          <Link
            href="/login"
            className="text-gray-400 hover:text-white text-sm transition-colors"
          >
            Sign In
          </Link>
          <Link
            href="/register"
            className="bg-teal-600 hover:bg-teal-500 text-white text-sm px-4 py-2 rounded-lg transition-colors"
          >
            Get Started Free
          </Link>
        </div>
      </nav>

      {/* Hero */}
      <section className="flex flex-col items-center text-center px-6 py-24 max-w-4xl mx-auto">
        <div className="inline-flex items-center gap-2 bg-teal-950 border border-teal-800 rounded-full px-4 py-1.5 text-teal-400 text-sm mb-8">
          <Shield className="w-3.5 h-3.5" aria-hidden="true" />
          AI-powered privacy protection
        </div>
        <h1 className="text-5xl sm:text-6xl font-bold text-white leading-tight mb-6">
          Your personal information.
          <br />
          <span className="text-teal-400">Under your control.</span>
        </h1>
        <p className="text-xl text-gray-400 mb-10 max-w-2xl">
          VectorScrub discovers where your personal data is exposed, identifies
          exactly what's out there, and helps remove it — automatically.
        </p>
        <div className="flex flex-col sm:flex-row gap-4">
          <Link
            href="/register"
            className="bg-teal-600 hover:bg-teal-500 text-white px-8 py-3 rounded-lg font-medium transition-colors text-lg"
          >
            See my exposure — it&apos;s free
          </Link>
          <Link
            href="#how-it-works"
            className="border border-gray-700 hover:border-gray-500 text-gray-300 px-8 py-3 rounded-lg font-medium transition-colors text-lg"
          >
            How it works
          </Link>
        </div>
        <p className="text-gray-600 text-sm mt-6">
          No credit card required for the free scan.
        </p>
      </section>

      {/* Features */}
      <section id="how-it-works" className="px-6 py-20 bg-gray-900/50">
        <div className="max-w-6xl mx-auto">
          <h2 className="text-3xl font-bold text-white text-center mb-4">
            Six AI agents working for you
          </h2>
          <p className="text-gray-400 text-center mb-16 max-w-xl mx-auto">
            VectorScrub runs a coordinated pipeline of specialized AI agents —
            not a single model doing everything.
          </p>
          <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-8">
            {features.map((f) => (
              <div
                key={f.title}
                className="bg-gray-800/60 border border-gray-700 rounded-xl p-6"
              >
                <f.icon
                  className="text-teal-400 w-7 h-7 mb-4"
                  aria-hidden="true"
                />
                <h3 className="text-white font-semibold text-lg mb-2">
                  {f.title}
                </h3>
                <p className="text-gray-400 text-sm leading-relaxed">
                  {f.description}
                </p>
              </div>
            ))}
          </div>
        </div>
      </section>

      {/* Pricing */}
      <section id="pricing" className="px-6 py-20">
        <div className="max-w-6xl mx-auto">
          <h2 className="text-3xl font-bold text-white text-center mb-4">
            Simple, transparent pricing
          </h2>
          <p className="text-gray-400 text-center mb-16 max-w-xl mx-auto">
            Start free. Upgrade when you&apos;re ready to start removing.
          </p>
          <div className="grid grid-cols-1 sm:grid-cols-2 gap-6 max-w-3xl mx-auto">
            {plans.map((plan) => (
              <div
                key={plan.name}
                className={`rounded-xl border p-6 flex flex-col ${
                  plan.highlight
                    ? "border-teal-500 bg-teal-950/40"
                    : "border-gray-700 bg-gray-800/40"
                }`}
              >
                {plan.badge && (
                  <div className="text-xs text-teal-400 font-semibold uppercase tracking-wider mb-3">
                    {plan.badge}
                  </div>
                )}
                <div className="text-white font-bold text-xl mb-1">
                  {plan.name}
                </div>
                <div className="flex items-end gap-1 mb-1">
                  <span className="text-3xl font-bold text-white">
                    {plan.price}
                  </span>
                  {plan.per && (
                    <span className="text-gray-500 text-sm pb-1">{plan.per}</span>
                  )}
                </div>
                <p className="text-gray-400 text-sm mb-2">{plan.description}</p>
                {plan.note && <p className="text-teal-300/80 text-xs mb-6">{plan.note}</p>}
                {!plan.note && <div className="mb-4" />}
                <ul className="space-y-2 mb-4">
                  {plan.features.map((f) => (
                    <li key={f} className="flex items-start gap-2 text-sm text-gray-300">
                      <span className="text-teal-400 mt-0.5">✓</span>
                      {f}
                    </li>
                  ))}
                </ul>
                <PlanReach tier={plan.tier} coverage={coverage} />
                <Link
                  href="/register"
                  className={`text-center py-2.5 rounded-lg text-sm font-medium transition-colors ${
                    plan.highlight
                      ? "bg-teal-600 hover:bg-teal-500 text-white"
                      : "border border-gray-600 hover:border-gray-400 text-gray-300"
                  }`}
                >
                  {plan.cta}
                </Link>
              </div>
            ))}
          </div>
          {overall.readable !== null && (
            <p className="text-gray-300 text-sm text-center mt-10 max-w-3xl mx-auto">
              In our recent scans we could read{" "}
              <span className="text-white font-semibold">
                {overall.readable} of the {overall.listed}
              </span>{" "}
              brokers we list. The others block automated tools or can only be searched by hand.
            </p>
          )}
          <p className="text-gray-500 text-sm text-center mt-4 max-w-3xl mx-auto">
            Many data brokers block automated tools. Your dashboard shows exactly which sites we could and
            could not check, and for the ones we cannot reach, our guide walks you through opting out by
            hand. Finding nothing on a site we could not check does not mean you are not listed there.
            California residents can also use the state&apos;s free DROP service, which our guide links to.
          </p>
        </div>
      </section>

      {/* Trust footer */}
      <section className="px-6 py-12 bg-gray-900/50 border-t border-gray-800">
        <div className="max-w-4xl mx-auto text-center">
          <p className="text-gray-500 text-sm">
            VectorScrub is a product of{" "}
            <span className="text-gray-400 font-medium">VectorGuard Labs</span>.
            Your data is encrypted with AES-256-GCM, never sold, and never used
            to train AI models. We are not affiliated with any data broker.
          </p>
        </div>
      </section>

      {/* Footer */}
      <footer className="px-6 py-6 border-t border-gray-800 text-center">
        <p className="text-gray-600 text-xs">
          © 2026 VectorGuard Labs. All rights reserved.
          &nbsp;·&nbsp;
          <Link href="/privacy" className="hover:text-gray-400 transition-colors">Privacy Policy</Link>
          &nbsp;·&nbsp;
          <Link href="/terms" className="hover:text-gray-400 transition-colors">Terms of Service</Link>
        </p>
      </footer>
    </main>
  );
}

/** What a plan's broker list means in practice, from recent scans (or general wording until measured). */
function PlanReach({
  tier,
  coverage,
}: {
  tier: PlanTier;
  coverage: Awaited<ReturnType<typeof fetchPublicCoverage>>;
}) {
  const { listed, readable } = planCoverage(tier, coverage);
  return (
    <p
      className="text-xs text-gray-400 border-t border-gray-700/60 pt-3 mb-6 flex-1"
      data-testid={`reach-${tier}`}
    >
      {readable === null ? (
        <>
          Some of these {listed} brokers block automated tools. Your dashboard shows exactly which ones we
          could and could not check.
        </>
      ) : (
        <>
          Right now we can read{" "}
          <span className="text-gray-200 font-medium">
            {readable} of these {listed}
          </span>{" "}
          automatically. For the rest, you get a guide to opt out by hand.
        </>
      )}
    </p>
  );
}
