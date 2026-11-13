"use client";

import { useState, FormEvent } from "react";
import { useRouter } from "next/navigation";
import Link from "next/link";
import { Shield } from "lucide-react";
import { register } from "@/lib/api";
import { TERMS_VERSION } from "@/lib/legal";

export default function RegisterPage() {
  const router = useRouter();
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(false);
  const [accepted, setAccepted] = useState(false);
  const [interviewOk, setInterviewOk] = useState(false);
  const [storyOk, setStoryOk] = useState(false);

  async function handleSubmit(e: FormEvent) {
    e.preventDefault();
    setError("");
    if (!accepted) {
      setError("Please read and accept the Terms of Service and Privacy Policy to continue.");
      return;
    }
    setLoading(true);
    try {
      await register(email, password, TERMS_VERSION, { interview: interviewOk, story: storyOk });
      router.push("/dashboard");
    } catch (err: unknown) {
      setError(err instanceof Error ? err.message : "Registration failed");
    } finally {
      setLoading(false);
    }
  }

  return (
    <main className="min-h-screen flex items-center justify-center px-4">
      <div className="w-full max-w-md">
        <div className="flex items-center justify-center gap-2 mb-8">
          <Shield className="text-teal-400 w-7 h-7" aria-hidden="true" />
          <span className="text-white font-bold text-xl">VectorScrub</span>
        </div>
        <div className="bg-gray-900 border border-gray-800 rounded-2xl p-8">
          <h1 className="text-2xl font-bold text-white mb-2">
            Create your account
          </h1>
          <p className="text-gray-400 text-sm mb-8">
            Free scan — no credit card required.
          </p>
          <form onSubmit={handleSubmit} className="space-y-5" noValidate>
            <div>
              <label
                htmlFor="email"
                className="block text-sm text-gray-400 mb-1.5"
              >
                Email address
              </label>
              <input
                id="email"
                type="email"
                autoComplete="email"
                required
                value={email}
                onChange={(e) => setEmail(e.target.value)}
                className="w-full bg-gray-800 border border-gray-700 rounded-lg px-4 py-2.5 text-white text-sm focus:outline-none focus:border-teal-500 transition-colors"
                placeholder="you@example.com"
              />
            </div>
            <div>
              <label
                htmlFor="password"
                className="block text-sm text-gray-400 mb-1.5"
              >
                Password
                <span className="text-gray-600 ml-1">(12+ characters)</span>
              </label>
              <input
                id="password"
                type="password"
                autoComplete="new-password"
                required
                minLength={12}
                value={password}
                onChange={(e) => setPassword(e.target.value)}
                className="w-full bg-gray-800 border border-gray-700 rounded-lg px-4 py-2.5 text-white text-sm focus:outline-none focus:border-teal-500 transition-colors"
                placeholder="Choose a strong password"
              />
            </div>
            <div className="text-xs text-gray-400 border border-gray-800 rounded-lg p-3 space-y-1">
              <p className="text-gray-300 font-medium">Before you start, this is an early pilot:</p>
              <ul className="list-disc ml-4 space-y-1">
                <li>We cannot remove you from every site. Many block automated tools.</li>
                <li>For those, you will do the opt-out yourself from our steps, then we check the listing.</li>
                <li>We submit a removal only after you authorize that listing.</li>
                <li>We mark a listing removed only when our own check confirms it.</li>
              </ul>
            </div>
            <div className="flex items-start gap-2">
              <input
                id="accept"
                type="checkbox"
                checked={accepted}
                onChange={(e) => setAccepted(e.target.checked)}
                className="mt-1"
              />
              <label htmlFor="accept" className="text-sm text-gray-400">
                I have read and accept the{" "}
                <Link href="/terms" target="_blank" className="text-teal-400 underline">
                  Terms of Service
                </Link>{" "}
                and{" "}
                <Link href="/privacy" target="_blank" className="text-teal-400 underline">
                  Privacy Policy
                </Link>
                .
              </label>
            </div>
            <fieldset className="space-y-2 border border-gray-800 rounded-lg p-3">
              <legend className="text-xs text-gray-400 px-1">Optional. Neither is needed to take part.</legend>
              <label className="flex items-start gap-2 text-xs text-gray-400">
                <input type="checkbox" checked={interviewOk} onChange={(e) => setInterviewOk(e.target.checked)} className="mt-0.5" />
                You may contact me about a short follow-up interview about my experience.
              </label>
              <label className="flex items-start gap-2 text-xs text-gray-400">
                <input type="checkbox" checked={storyOk} onChange={(e) => setStoryOk(e.target.checked)} className="mt-0.5" />
                You may ask me whether you can share an anonymized version of my results. (Asking is not
                permission to publish; I would be asked first.)
              </label>
            </fieldset>
            {error && (
              <p role="alert" className="text-red-400 text-sm">
                {error}
              </p>
            )}
            <button
              type="submit"
              disabled={loading}
              className="w-full bg-teal-600 hover:bg-teal-500 disabled:opacity-50 disabled:cursor-not-allowed text-white py-3 rounded-lg font-medium transition-colors"
            >
              {loading ? "Creating account…" : "Create account"}
            </button>
          </form>
          <p className="text-gray-500 text-sm text-center mt-6">
            Already have an account?{" "}
            <Link
              href="/login"
              className="text-teal-400 hover:text-teal-300 transition-colors"
            >
              Sign in
            </Link>
          </p>
        </div>
      </div>
    </main>
  );
}
