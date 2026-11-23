"use client";

import { useCallback, useEffect, useState, FormEvent } from "react";
import { useRouter } from "next/navigation";
import { ArrowLeft } from "lucide-react";
import {
  changePassword,
  disableMfa,
  enableMfa,
  endSession,
  getMfaStatus,
  getSessions,
  logoutEverywhere,
  startMfaSetup,
} from "@/lib/api";
import { formatDateTime } from "@/lib/utils";
import type { MfaSetup, MfaStatus, SessionInfo } from "@/types";

const INPUT =
  "w-full max-w-sm bg-gray-800 border border-gray-700 rounded-lg px-3 py-2 text-white text-sm focus:outline-none focus:border-brand-blue-500";
const PRIMARY =
  "px-4 py-2 bg-brand-red-600 hover:bg-brand-red-500 disabled:opacity-50 disabled:cursor-not-allowed text-white rounded-lg text-sm font-medium transition-colors";
const SECONDARY = "px-4 py-2 border border-gray-700 text-gray-300 hover:text-white rounded-lg text-sm";

/** A short, readable description of a browser from its User-Agent string. */
function describeAgent(ua: string | null): string {
  if (!ua) return "Unknown device";
  const browser = /Edg\//.test(ua)
    ? "Edge"
    : /Chrome\//.test(ua)
      ? "Chrome"
      : /Firefox\//.test(ua)
        ? "Firefox"
        : /Safari\//.test(ua)
          ? "Safari"
          : "Browser";
  const os = /Windows/.test(ua)
    ? "Windows"
    : /iPhone|iPad/.test(ua)
      ? "iOS"
      : /Android/.test(ua)
        ? "Android"
        : /Mac OS X/.test(ua)
          ? "macOS"
          : /Linux/.test(ua)
            ? "Linux"
            : "";
  return os ? `${browser} on ${os}` : browser;
}

export default function SecurityPage() {
  const router = useRouter();
  const [mfa, setMfa] = useState<MfaStatus | null>(null);
  const [sessions, setSessions] = useState<SessionInfo[]>([]);
  const [loadError, setLoadError] = useState("");

  const refresh = useCallback(async () => {
    try {
      const [m, s] = await Promise.all([getMfaStatus(), getSessions()]);
      setMfa(m);
      setSessions(s);
      setLoadError("");
    } catch (e) {
      setLoadError(e instanceof Error ? e.message : "Could not load your security settings.");
    }
  }, []);

  useEffect(() => {
    let live = true;
    Promise.all([getMfaStatus(), getSessions()])
      .then(([m, s]) => {
        if (!live) return;
        setMfa(m);
        setSessions(s);
        setLoadError("");
      })
      .catch((e) => {
        if (live) setLoadError(e instanceof Error ? e.message : "Could not load your security settings.");
      });
    return () => {
      live = false;
    };
  }, []);

  return (
    <main className="min-h-screen px-6 py-10">
      <div className="max-w-2xl mx-auto">
        <button
          onClick={() => router.push("/dashboard")}
          className="flex items-center gap-1.5 text-gray-400 hover:text-white text-sm mb-6"
        >
          <ArrowLeft className="w-4 h-4" /> Back to dashboard
        </button>
        <h1 className="text-3xl font-bold text-white mb-2">Security</h1>
        <p className="text-gray-400 text-sm mb-8">
          Protect your account with a second sign-in step, and see where you are signed in.
        </p>

        {loadError && (
          <p role="alert" className="text-red-400 text-sm mb-6">
            {loadError}
          </p>
        )}

        <PasswordSection mfaEnabled={Boolean(mfa?.enabled)} onChanged={refresh} />
        {mfa && <MfaSection status={mfa} onChanged={refresh} />}
        <SessionsSection sessions={sessions} onChanged={refresh} />
      </div>
    </main>
  );
}

function MfaSection({ status, onChanged }: { status: MfaStatus; onChanged: () => Promise<void> }) {
  const [step, setStep] = useState<"idle" | "password" | "scan" | "codes" | "disable">("idle");
  const [password, setPassword] = useState("");
  const [code, setCode] = useState("");
  const [setup, setSetup] = useState<MfaSetup | null>(null);
  const [recovery, setRecovery] = useState<string[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");

  function reset() {
    setStep("idle");
    setPassword("");
    setCode("");
    setSetup(null);
    setError("");
  }

  async function run(action: () => Promise<void>) {
    setBusy(true);
    setError("");
    try {
      await action();
    } catch (e) {
      setError(e instanceof Error ? e.message : "Something went wrong. Try again.");
    } finally {
      setBusy(false);
    }
  }

  const begin = (e: FormEvent) => {
    e.preventDefault();
    void run(async () => {
      setSetup(await startMfaSetup(password));
      setPassword("");
      setStep("scan");
    });
  };

  const confirm = (e: FormEvent) => {
    e.preventDefault();
    void run(async () => {
      setRecovery(await enableMfa(code.trim()));
      setCode("");
      setSetup(null);
      setStep("codes");
      await onChanged();
    });
  };

  const turnOff = (e: FormEvent) => {
    e.preventDefault();
    void run(async () => {
      await disableMfa(password, code.trim());
      reset();
      await onChanged();
    });
  };

  return (
    <section className="bg-gray-900 border border-gray-800 rounded-2xl p-6 mb-8" data-testid="mfa-section">
      <div className="flex items-start justify-between gap-4 mb-2">
        <h2 className="text-lg font-semibold text-white">Two-factor authentication</h2>
        <span
          className={`text-xs px-2 py-1 rounded-full ${
            status.enabled ? "bg-green-900/40 text-green-300" : "bg-gray-800 text-gray-400"
          }`}
        >
          {status.enabled ? "On" : "Off"}
        </span>
      </div>
      <p className="text-gray-400 text-sm mb-4">
        With two-factor on, signing in needs a 6-digit code from an authenticator app (such as Google
        Authenticator, Authy, 1Password or Aegis) as well as your password. Someone who learns your
        password still cannot get in.
      </p>

      {step === "codes" ? (
        <div className="space-y-3">
          <p className="text-green-300 text-sm font-medium">Two-factor authentication is on.</p>
          <p className="text-gray-300 text-sm">
            Save these recovery codes somewhere safe, such as a password manager. Each works once if you
            lose your phone. <strong>They are shown only now.</strong> Your other devices were signed out.
          </p>
          <ul className="grid grid-cols-2 gap-2 font-mono text-sm text-white bg-gray-800 rounded-lg p-4" data-testid="recovery-codes">
            {recovery.map((c) => (
              <li key={c}>{c}</li>
            ))}
          </ul>
          <button type="button" className={PRIMARY} onClick={() => { setRecovery([]); reset(); }}>
            I have saved them
          </button>
        </div>
      ) : step === "idle" ? (
        status.enabled ? (
          <div className="space-y-3">
            <p className="text-gray-400 text-sm">
              Recovery codes left: <span className="text-white">{status.recovery_codes_remaining}</span>
              {status.recovery_codes_remaining <= 2 && (
                <span className="text-yellow-300"> (running low: turn two-factor off and on again for new ones)</span>
              )}
            </p>
            <button type="button" className={SECONDARY} onClick={() => setStep("disable")}>
              Turn off…
            </button>
          </div>
        ) : (
          <button type="button" className={PRIMARY} onClick={() => setStep("password")}>
            Turn on…
          </button>
        )
      ) : step === "password" ? (
        <form onSubmit={begin} className="space-y-3">
          <label htmlFor="mfa-password" className="block text-xs text-gray-400">
            Enter your password to continue
          </label>
          <input id="mfa-password" type="password" autoComplete="current-password" required value={password}
            onChange={(e) => setPassword(e.target.value)} className={INPUT} />
          <div className="flex gap-2">
            <button type="submit" disabled={busy || !password} className={PRIMARY}>{busy ? "Checking…" : "Continue"}</button>
            <button type="button" className={SECONDARY} onClick={reset}>Cancel</button>
          </div>
        </form>
      ) : step === "scan" && setup ? (
        <form onSubmit={confirm} className="space-y-3">
          <ol className="list-decimal list-inside text-sm text-gray-300 space-y-1">
            <li>
              On this phone, <a href={setup.otpauth_uri} className="underline text-brand-blue-400">open this link</a> to
              add the account to your authenticator app. Or add it by hand with this key:
            </li>
          </ol>
          <p className="font-mono text-sm text-white bg-gray-800 rounded-lg px-3 py-2 break-all select-all" data-testid="mfa-secret">
            {setup.secret.match(/.{1,4}/g)?.join(" ")}
          </p>
          <p className="text-sm text-gray-300">Then enter the 6-digit code the app shows:</p>
          <input id="mfa-confirm" aria-label="Verification code" type="text" inputMode="numeric"
            autoComplete="one-time-code" required value={code} onChange={(e) => setCode(e.target.value)} className={INPUT} />
          <div className="flex gap-2">
            <button type="submit" disabled={busy || !code} className={PRIMARY}>{busy ? "Checking…" : "Turn on"}</button>
            <button type="button" className={SECONDARY} onClick={reset}>Cancel</button>
          </div>
        </form>
      ) : step === "disable" ? (
        <form onSubmit={turnOff} className="space-y-3">
          <label htmlFor="mfa-off-password" className="block text-xs text-gray-400">Password</label>
          <input id="mfa-off-password" type="password" autoComplete="current-password" required value={password}
            onChange={(e) => setPassword(e.target.value)} className={INPUT} />
          <label htmlFor="mfa-off-code" className="block text-xs text-gray-400">Code from your app, or a recovery code</label>
          <input id="mfa-off-code" type="text" autoComplete="one-time-code" required value={code}
            onChange={(e) => setCode(e.target.value)} className={INPUT} />
          <div className="flex gap-2">
            <button type="submit" disabled={busy || !password || !code} className={PRIMARY}>{busy ? "Turning off…" : "Turn off"}</button>
            <button type="button" className={SECONDARY} onClick={reset}>Cancel</button>
          </div>
        </form>
      ) : null}

      {error && (
        <p role="alert" className="text-red-400 text-sm mt-3">
          {error}
        </p>
      )}
    </section>
  );
}

function SessionsSection({ sessions, onChanged }: { sessions: SessionInfo[]; onChanged: () => Promise<void> }) {
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);

  async function end(id: string) {
    setBusy(true);
    setError("");
    try {
      await endSession(id);
      await onChanged();
    } catch (e) {
      setError(e instanceof Error ? e.message : "Could not end that session.");
    } finally {
      setBusy(false);
    }
  }

  async function everywhere() {
    setBusy(true);
    setError("");
    try {
      await logoutEverywhere();
      // A full page load on purpose: it drops every piece of client state from the signed-out session.
      // eslint-disable-next-line @next/next/no-location-assign-relative-destination -- full reload is intended here
      window.location.href = "/login";
    } catch (e) {
      setError(e instanceof Error ? e.message : "Could not sign out everywhere.");
      setBusy(false);
    }
  }

  return (
    <section className="bg-gray-900 border border-gray-800 rounded-2xl p-6" data-testid="sessions-section">
      <h2 className="text-lg font-semibold text-white mb-2">Where you are signed in</h2>
      <p className="text-gray-400 text-sm mb-4">
        Sessions end on their own after an hour without use, and after 24 hours at most.
      </p>
      <ul className="divide-y divide-gray-800 mb-4">
        {sessions.map((s) => (
          <li key={s.id} className="py-3 flex items-center justify-between gap-4">
            <div>
              <p className="text-white text-sm">
                {describeAgent(s.user_agent)}
                {s.current && <span className="ml-2 text-xs text-green-300">This device</span>}
              </p>
              <p className="text-gray-500 text-xs">
                Signed in {formatDateTime(s.created_at)} · last used {formatDateTime(s.last_seen_at)}
              </p>
            </div>
            {!s.current && (
              <button type="button" disabled={busy} className={SECONDARY} onClick={() => void end(s.id)}>
                Sign out
              </button>
            )}
          </li>
        ))}
      </ul>
      <button type="button" disabled={busy} className={SECONDARY} onClick={() => void everywhere()}>
        Sign out everywhere
      </button>
      {error && (
        <p role="alert" className="text-red-400 text-sm mt-3">
          {error}
        </p>
      )}
    </section>
  );
}

function PasswordSection({ mfaEnabled, onChanged }: { mfaEnabled: boolean; onChanged: () => Promise<void> }) {
  const [current, setCurrent] = useState("");
  const [next, setNext] = useState("");
  const [again, setAgain] = useState("");
  const [code, setCode] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [done, setDone] = useState(false);

  async function submit(e: FormEvent) {
    e.preventDefault();
    setError("");
    setDone(false);
    if (next !== again) {
      setError("The two new passwords are not the same.");
      return;
    }
    if (next.length < 12) {
      setError("Use at least 12 characters. A few random words make a good password.");
      return;
    }
    setBusy(true);
    try {
      await changePassword(current, next, code.trim() || undefined);
      setCurrent("");
      setNext("");
      setAgain("");
      setCode("");
      setDone(true);
      await onChanged();
    } catch (err) {
      setError(err instanceof Error ? err.message : "Could not change the password. Try again.");
    } finally {
      setBusy(false);
    }
  }

  return (
    <section className="bg-gray-900 border border-gray-800 rounded-2xl p-6 mb-8" data-testid="password-section">
      <h2 className="text-lg font-semibold text-white mb-2">Password</h2>
      <p className="text-gray-400 text-sm mb-4">
        Changing it signs you out everywhere else.
      </p>
      <form onSubmit={submit} className="space-y-3">
        <label htmlFor="pw-current" className="block text-xs text-gray-400">Current password</label>
        <input id="pw-current" type="password" autoComplete="current-password" required value={current}
          onChange={(e) => setCurrent(e.target.value)} className={INPUT} />
        <label htmlFor="pw-new" className="block text-xs text-gray-400">New password (at least 12 characters)</label>
        <input id="pw-new" type="password" autoComplete="new-password" required minLength={12} value={next}
          onChange={(e) => setNext(e.target.value)} className={INPUT} />
        <label htmlFor="pw-again" className="block text-xs text-gray-400">New password again</label>
        <input id="pw-again" type="password" autoComplete="new-password" required value={again}
          onChange={(e) => setAgain(e.target.value)} className={INPUT} />
        {mfaEnabled && (
          <>
            <label htmlFor="pw-code" className="block text-xs text-gray-400">Code from your authenticator app</label>
            <input id="pw-code" type="text" inputMode="numeric" autoComplete="one-time-code" required value={code}
              onChange={(e) => setCode(e.target.value)} className={INPUT} />
          </>
        )}
        <button type="submit" disabled={busy || !current || !next || !again} className={PRIMARY}>
          {busy ? "Changing…" : "Change password"}
        </button>
        {done && <p role="status" className="text-green-300 text-sm">Password changed. Your other sessions were signed out.</p>}
        {error && <p role="alert" className="text-red-400 text-sm">{error}</p>}
      </form>
    </section>
  );
}
