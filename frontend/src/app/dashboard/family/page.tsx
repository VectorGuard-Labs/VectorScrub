"use client";

import { useEffect, useState } from "react";
import { useRouter } from "next/navigation";
import {
  Shield,
  LogOut,
  Users,
  UserPlus,
  UserMinus,
  TrendingUp,
  TrendingDown,
  Crown,
  User,
  Mail,
  AlertTriangle,
} from "lucide-react";
import {
  getFamilyStats,
  addFamilyMember,
  removeFamilyMember,
  getProfile,
} from "@/lib/api";
import { privacyScoreLabel } from "@/lib/utils";
import type { FamilyStats, FamilyMember } from "@/lib/api";
import type { CustomerProfile } from "@/types";

export default function FamilyPage() {
  const router = useRouter();
  const [profile, setProfile] = useState<CustomerProfile | null>(null);
  const [stats, setStats] = useState<FamilyStats | null>(null);
  const [loading, setLoading] = useState(true);
  const [addLoading, setAddLoading] = useState(false);
  const [newMemberEmail, setNewMemberEmail] = useState("");
  const [message, setMessage] = useState("");
  const [error, setError] = useState("");

  useEffect(() => {
    const token = localStorage.getItem("vs_token");
    if (!token) {
      router.push("/login");
      return;
    }
    loadData();
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  async function loadData() {
    setLoading(true);
    try {
      const [p, s] = await Promise.all([getProfile(), getFamilyStats()]);
      setProfile(p);
      setStats(s);

      // Check if user has family plan
      if (p.subscription_tier !== "family") {
        setError("Family dashboard is only available for Family plan subscribers.");
      }
    } catch (err) {
      setError("Failed to load family dashboard. Please try again.");
    } finally {
      setLoading(false);
    }
  }

  async function handleAddMember() {
    if (!newMemberEmail.trim()) return;

    setAddLoading(true);
    setMessage("");
    setError("");

    try {
      const result = await addFamilyMember(newMemberEmail);
      setMessage(`Family member added: ${newMemberEmail}`);
      setNewMemberEmail("");
      await loadData();
    } catch (err: unknown) {
      setError(err instanceof Error ? err.message : "Failed to add member");
    } finally {
      setAddLoading(false);
    }
  }

  async function handleRemoveMember(memberId: string, memberEmail: string) {
    if (!confirm(`Remove ${memberEmail} from your family plan?`)) return;

    try {
      await removeFamilyMember(memberId);
      setMessage(`Removed ${memberEmail} from family`);
      await loadData();
    } catch (err: unknown) {
      setError(err instanceof Error ? err.message : "Failed to remove member");
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
          Loading family dashboard…
        </div>
      </div>
    );
  }

  return (
    <div className="min-h-screen">
      {/* Top nav */}
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
            onClick={() => router.push("/dashboard/profile")}
            className="text-gray-400 hover:text-white text-sm font-medium transition-colors"
          >
            Identity
          </button>
          <button
            onClick={() => router.push("/dashboard/removals")}
            className="text-gray-400 hover:text-white text-sm font-medium transition-colors"
          >
            Removals
          </button>
          <button
            onClick={() => router.push("/dashboard/history")}
            className="text-gray-400 hover:text-white text-sm font-medium transition-colors"
          >
            History
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

      <main className="max-w-5xl mx-auto px-6 py-10">
        {/* Header */}
        <div className="mb-8">
          <div className="flex items-center gap-2 mb-2">
            <Users className="w-6 h-6 text-teal-400" />
            <h1 className="text-2xl font-bold text-white">Family Dashboard</h1>
          </div>
          <p className="text-gray-400 text-sm">
            Manage up to 5 family members under your Family plan
          </p>
        </div>

        {/* Messages */}
        {error && (
          <div
            role="alert"
            className="bg-red-950 border border-red-800 text-red-300 rounded-lg px-4 py-3 mb-6 text-sm"
          >
            {error}
          </div>
        )}

        {message && (
          <div className="bg-teal-950 border border-teal-800 text-teal-300 rounded-lg px-4 py-3 mb-6 text-sm">
            {message}
          </div>
        )}

        {/* Check if user has family plan */}
        {profile?.subscription_tier !== "family" ? (
          <div className="bg-gray-900 border border-gray-800 rounded-xl p-12 text-center">
            <Users className="w-12 h-12 text-gray-600 mx-auto mb-4" />
            <h2 className="text-xl font-bold text-white mb-2">
              Family Plan Required
            </h2>
            <p className="text-gray-400 mb-6">
              Upgrade to the Family plan to protect up to 5 family members with
              shared dashboard and monitoring.
            </p>
            <button className="bg-teal-600 hover:bg-teal-500 text-white px-6 py-2.5 rounded-lg text-sm font-medium transition-colors">
              Upgrade to Family Plan
            </button>
          </div>
        ) : (
          <>
            {/* Family stats */}
            {stats && (
              <div className="grid grid-cols-1 sm:grid-cols-3 gap-4 mb-10">
                <div className="bg-gray-900 border border-gray-800 rounded-xl p-5">
                  <div className="text-3xl font-bold text-white mb-1">
                    {stats.total_members}
                  </div>
                  <div className="text-gray-400 text-sm">Family Members</div>
                  <div className="text-xs text-gray-500 mt-1">
                    {5 - stats.total_members} slots remaining
                  </div>
                </div>
                <div className="bg-gray-900 border border-gray-800 rounded-xl p-5">
                  <div
                    className={`text-3xl font-bold mb-1 ${
                      privacyScoreLabel(stats.average_privacy_score).color
                    }`}
                  >
                    {Math.round(stats.average_privacy_score)}
                  </div>
                  <div className="text-gray-400 text-sm">
                    Average Privacy Score
                  </div>
                  <div className="text-xs text-gray-500 mt-1">
                    {privacyScoreLabel(stats.average_privacy_score).label}
                  </div>
                </div>
                <div className="bg-gray-900 border border-gray-800 rounded-xl p-5">
                  <div className="text-3xl font-bold text-orange-400 mb-1">
                    {stats.total_exposures}
                  </div>
                  <div className="text-gray-400 text-sm">Total Exposures</div>
                  <div className="text-xs text-gray-500 mt-1">
                    Across all members
                  </div>
                </div>
              </div>
            )}

            {/* Add member form */}
            {stats && stats.total_members < 5 && (
              <div className="bg-gray-900 border border-gray-800 rounded-xl p-6 mb-8">
                <h2 className="text-lg font-semibold text-white mb-4 flex items-center gap-2">
                  <UserPlus className="w-5 h-5 text-teal-400" />
                  Add Family Member
                </h2>
                <div className="flex gap-3">
                  <input
                    type="email"
                    value={newMemberEmail}
                    onChange={(e) => setNewMemberEmail(e.target.value)}
                    placeholder="family-member@example.com"
                    className="flex-1 bg-gray-800 border border-gray-700 text-white rounded-lg px-4 py-2.5 text-sm focus:ring-2 focus:ring-teal-600 focus:outline-none"
                    onKeyDown={(e) => {
                      if (e.key === "Enter") handleAddMember();
                    }}
                  />
                  <button
                    onClick={handleAddMember}
                    disabled={addLoading || !newMemberEmail.trim()}
                    className="bg-teal-600 hover:bg-teal-500 disabled:opacity-50 disabled:cursor-not-allowed text-white px-5 py-2.5 rounded-lg text-sm font-medium transition-colors"
                  >
                    {addLoading ? "Adding…" : "Add Member"}
                  </button>
                </div>
                <p className="text-gray-500 text-xs mt-2">
                  They must have an existing VectorScrub account
                </p>
              </div>
            )}

            {/* Family members list */}
            <div>
              <h2 className="text-lg font-semibold text-white mb-4">
                Family Members
              </h2>
              {stats && stats.members.length === 0 ? (
                <div className="bg-gray-900 border border-gray-800 rounded-xl p-12 text-center">
                  <Users
                    className="w-10 h-10 text-gray-600 mx-auto mb-3"
                    aria-hidden="true"
                  />
                  <p className="text-gray-400 mb-1">No family members yet</p>
                  <p className="text-gray-600 text-sm">
                    Add your first family member using the form above
                  </p>
                </div>
              ) : (
                <div className="space-y-3">
                  {stats?.members.map((member: FamilyMember) => {
                    const scoreInfo = member.privacy_score
                      ? privacyScoreLabel(member.privacy_score)
                      : { label: "No score", color: "text-gray-500" };

                    return (
                      <div
                        key={member.id}
                        className="bg-gray-900 border border-gray-800 rounded-xl p-5 flex items-center gap-4"
                      >
                        <div
                          className={`flex-shrink-0 w-12 h-12 rounded-full flex items-center justify-center ${
                            member.role === "owner"
                              ? "bg-amber-950 border border-amber-800"
                              : "bg-gray-800 border border-gray-700"
                          }`}
                        >
                          {member.role === "owner" ? (
                            <Crown className="w-5 h-5 text-amber-400" />
                          ) : (
                            <User className="w-5 h-5 text-gray-400" />
                          )}
                        </div>
                        <div className="flex-1 min-w-0">
                          <div className="flex items-center gap-2 mb-1">
                            <Mail className="w-4 h-4 text-gray-500" />
                            <span className="text-white font-medium text-sm">
                              {member.member_email}
                            </span>
                            {member.role === "owner" && (
                              <span className="text-xs bg-amber-950 border border-amber-800 text-amber-400 px-2 py-0.5 rounded-full font-medium">
                                Owner
                              </span>
                            )}
                          </div>
                          <div className="flex items-center gap-4 text-xs text-gray-400">
                            <div>
                              Privacy Score:{" "}
                              <span className={`font-medium ${scoreInfo.color}`}>
                                {member.privacy_score
                                  ? Math.round(member.privacy_score)
                                  : "—"}
                              </span>
                            </div>
                            {member.invited_at && !member.accepted_at && (
                              <span className="flex items-center gap-1 text-orange-400">
                                <AlertTriangle className="w-3 h-3" />
                                Pending invitation
                              </span>
                            )}
                          </div>
                        </div>
                        {member.role !== "owner" && (
                          <button
                            onClick={() =>
                              handleRemoveMember(member.id, member.member_email)
                            }
                            className="flex-shrink-0 text-red-400 hover:text-red-300 transition-colors"
                            aria-label="Remove member"
                          >
                            <UserMinus className="w-5 h-5" />
                          </button>
                        )}
                      </div>
                    );
                  })}
                </div>
              )}
            </div>
          </>
        )}
      </main>
    </div>
  );
}
