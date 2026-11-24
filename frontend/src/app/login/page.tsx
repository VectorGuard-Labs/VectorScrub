"use client";

import { useState, FormEvent } from "react";
import { useRouter } from "next/navigation";
import Link from "next/link";
import { login, needsMfa } from "@/lib/api";

export default function LoginPage() {
  const router = useRouter();
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  // Set once the password was right and the account asks for a second factor.
  const [needCode, setNeedCode] = useState(false);
  const [code, setCode] = useState("");
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(false);

  async function handleSubmit(e: FormEvent) {
    e.preventDefault();
    setError("");
    setLoading(true);
    try {
      const result = await login(email, password, needCode ? code.trim() : undefined);
      if (needsMfa(result)) {
        setNeedCode(true);
        return;
      }
      router.push("/dashboard");
    } catch (err: unknown) {
      setError(err instanceof Error ? err.message : "Sign in failed");
    } finally {
      setLoading(false);
    }
  }

  return (
    <main className="flex-1 flex items-center justify-center px-4">
      <div className="w-full max-w-md">
        <div className="flex items-center justify-center gap-2 mb-8">
          <span className="text-white font-bold text-xl">VectorScrub</span>
        </div>
        <div className="bg-gray-900 border border-gray-800 rounded-2xl p-8">
          <h1 className="text-2xl font-bold text-white mb-2">Welcome back</h1>
          <p className="text-gray-400 text-sm mb-8">
            Sign in to your VectorScrub account.
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
                className="w-full bg-gray-800 border border-gray-700 rounded-lg px-4 py-2.5 text-white text-sm focus:outline-none focus:border-brand-blue-500 transition-colors"
                placeholder="you@example.com"
              />
            </div>
            <div>
              <label
                htmlFor="password"
                className="block text-sm text-gray-400 mb-1.5"
              >
                Password
              </label>
              <input
                id="password"
                type="password"
                autoComplete="current-password"
                required
                value={password}
                onChange={(e) => setPassword(e.target.value)}
                className="w-full bg-gray-800 border border-gray-700 rounded-lg px-4 py-2.5 text-white text-sm focus:outline-none focus:border-brand-blue-500 transition-colors"
                placeholder="Your password"
              />
            </div>
            {needCode && (
              <div>
                <label
                  htmlFor="mfa-code"
                  className="block text-sm text-gray-400 mb-1.5"
                >
                  Verification code
                </label>
                <input
                  id="mfa-code"
                  type="text"
                  inputMode="numeric"
                  autoComplete="one-time-code"
                  autoFocus
                  required
                  value={code}
                  onChange={(e) => setCode(e.target.value)}
                  className="w-full bg-gray-800 border border-gray-700 rounded-lg px-4 py-2.5 text-white text-sm focus:outline-none focus:border-brand-blue-500 transition-colors"
                  placeholder="6-digit code or recovery code"
                />
                <p className="text-gray-500 text-xs mt-1.5">
                  Open your authenticator app and enter the current code. Lost your phone? Use one of
                  your recovery codes.
                </p>
              </div>
            )}
            {error && (
              <p role="alert" className="text-red-400 text-sm">
                {error}
              </p>
            )}
            <button
              type="submit"
              disabled={loading}
              className="w-full bg-brand-red-600 hover:bg-brand-red-500 disabled:opacity-50 disabled:cursor-not-allowed text-white py-3 rounded-lg font-medium transition-colors"
            >
              {loading ? "Signing in…" : "Sign in"}
            </button>
          </form>
          <p className="text-gray-500 text-sm text-center mt-6">
            No account?{" "}
            <Link
              href="/register"
              className="text-brand-blue-400 hover:text-brand-blue-300 transition-colors"
            >
              Create one free
            </Link>
          </p>
        </div>
      </div>
    </main>
  );
}
