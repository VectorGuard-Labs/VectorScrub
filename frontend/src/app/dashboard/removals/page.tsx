"use client";

import { useEffect, useState } from "react";
import { useRouter } from "next/navigation";
import { Shield, LogOut, ArrowLeft } from "lucide-react";
import { listRemovalRequests } from "@/lib/api";
import { removalStatusLabel, removalStatusColor, formatDate } from "@/lib/utils";
import type { RemovalRequest } from "@/types";

export default function RemovalsPage() {
  const router = useRouter();
  const [requests, setRequests] = useState<RemovalRequest[]>([]);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    const token = localStorage.getItem("vs_token");
    if (!token) {
      router.push("/login");
      return;
    }
    loadRequests();
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  async function loadRequests() {
    setLoading(true);
    try {
      const data = await listRemovalRequests();
      setRequests(data.requests);
    } finally {
      setLoading(false);
    }
  }

  function handleSignOut() {
    localStorage.removeItem("vs_token");
    router.push("/");
  }

  if (loading) {
    return (
      <div className="min-h-screen flex items-center justify-center">
        <div className="text-gray-400 text-sm animate-pulse">
          Loading removal requests…
        </div>
      </div>
    );
  }

  return (
    <div className="min-h-screen">
      {/* Top nav */}
      <nav className="border-b border-gray-800 bg-black/40 backdrop-blur-sm sticky top-0 z-10">
        <div className="max-w-6xl mx-auto px-6 py-4 flex items-center justify-between">
          <div className="flex items-center gap-2">
            <Shield className="w-6 h-6 text-teal-400" aria-hidden="true" />
            <span className="font-bold text-lg text-white">VectorScrub</span>
          </div>
          <button
            onClick={handleSignOut}
            className="flex items-center gap-2 text-gray-400 hover:text-white transition-colors text-sm"
          >
            <LogOut className="w-4 h-4" />
            Sign Out
          </button>
        </div>
      </nav>

      <main className="max-w-6xl mx-auto px-6 py-10">
        <div className="flex items-center gap-4 mb-6">
          <button
            onClick={() => router.push("/dashboard")}
            className="text-gray-400 hover:text-white transition-colors"
          >
            <ArrowLeft className="w-5 h-5" />
          </button>
          <h1 className="text-2xl font-bold text-white">Removal Requests</h1>
        </div>

        {requests.length === 0 ? (
          <div className="bg-gray-900 border border-gray-800 rounded-xl p-12 text-center">
            <p className="text-gray-400 mb-1">No removal requests yet.</p>
            <p className="text-gray-600 text-sm">
              Authorize removals from your dashboard to track their progress here.
            </p>
          </div>
        ) : (
          <div className="space-y-4">
            {requests.map((req) => (
              <div
                key={req.id}
                className="bg-gray-900 border border-gray-800 rounded-xl p-5"
              >
                <div className="flex items-center justify-between mb-3">
                  <span className="text-white font-medium">{req.source_name}</span>
                  <span
                    className={`text-xs px-3 py-1 rounded-full border ${removalStatusColor(
                      req.status
                    )}`}
                  >
                    {removalStatusLabel(req.status)}
                  </span>
                </div>

                <div className="text-gray-400 text-sm space-y-1">
                  <div>
                    Authorized: {formatDate(req.customer_authorized_at)}
                  </div>
                  {req.submitted_at && (
                    <div>Submitted: {formatDate(req.submitted_at)}</div>
                  )}
                  {req.confirmation_ref && (
                    <div>Confirmation: {req.confirmation_ref}</div>
                  )}
                  {req.next_check_at && (
                    <div>Next check: {formatDate(req.next_check_at)}</div>
                  )}
                  {req.verified_at && (
                    <div className="text-green-400">
                      Verified removed: {formatDate(req.verified_at)}
                    </div>
                  )}
                </div>
              </div>
            ))}
          </div>
        )}
      </main>
    </div>
  );
}
