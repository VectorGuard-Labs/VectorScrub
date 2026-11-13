"use client";

import { useState } from "react";
import { deleteAccount } from "@/lib/api";

/**
 * Permanent account erasure. Asks for the password again; the server refuses while a
 * subscription would keep billing and says so. What is erased and what is kept is the Privacy
 * Policy's section 7, linked here.
 */
export default function DeleteAccount() {
  const [open, setOpen] = useState(false);
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");

  async function confirm() {
    setBusy(true);
    setError("");
    try {
      await deleteAccount(password);
      // The session cookie is already cleared; leave the app.
      window.location.assign("/?deleted=1");
    } catch (e) {
      setError(e instanceof Error ? e.message : "Could not delete the account. Try again.");
      setBusy(false);
    }
  }

  return (
    <div className="mt-8 p-4 border border-red-900/60 rounded-lg" data-testid="delete-account">
      <h3 className="text-sm font-semibold text-red-300 mb-2">Delete my account</h3>
      <p className="text-xs text-gray-400 mb-3">
        This permanently erases your account, identity details, scans, exposures and removal requests. It
        cannot be undone. It does <strong>not</strong> remove your listings from broker sites. A security
        record of what was done (with your name, email and IP address removed) is kept. Cancel any
        paid plan on the Billing page first. See{" "}
        <a href="/privacy" target="_blank" className="underline">
          what is erased and what is kept
        </a>
        .
      </p>
      {!open ? (
        <button
          type="button"
          onClick={() => setOpen(true)}
          className="px-4 py-2 border border-red-700 text-red-300 hover:bg-red-950/40 rounded-lg text-sm"
        >
          Delete my account…
        </button>
      ) : (
        <div className="space-y-2">
          <label htmlFor="delete-password" className="block text-xs text-gray-400">
            Enter your password to confirm
          </label>
          <input
            id="delete-password"
            type="password"
            autoComplete="current-password"
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            className="w-full max-w-sm bg-gray-800 border border-gray-700 rounded-lg px-3 py-2 text-white text-sm"
          />
          {error && (
            <p role="alert" className="text-red-400 text-xs">
              {error}
            </p>
          )}
          <div className="flex gap-2">
            <button
              type="button"
              onClick={confirm}
              disabled={busy || password.length === 0}
              className="px-4 py-2 bg-red-700 hover:bg-red-600 disabled:opacity-50 text-white rounded-lg text-sm"
            >
              {busy ? "Deleting…" : "Permanently delete"}
            </button>
            <button
              type="button"
              onClick={() => {
                setOpen(false);
                setPassword("");
                setError("");
              }}
              className="px-4 py-2 border border-gray-700 text-gray-300 rounded-lg text-sm"
            >
              Keep my account
            </button>
          </div>
        </div>
      )}
    </div>
  );
}
