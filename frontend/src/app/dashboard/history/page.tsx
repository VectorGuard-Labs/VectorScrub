"use client";

import { useEffect, useState } from "react";
import { useRouter } from "next/navigation";
import {
  Shield,
  LogOut,
  AlertTriangle,
  CheckCircle,
  Send,
  RefreshCcw,
  Calendar,
  Filter,
  Download,
} from "lucide-react";
import { listExposures, listRemovalRequests, logout } from "@/lib/api";
import { formatDate, severityColor, piiCategoryLabel } from "@/lib/utils";
import type { Exposure, RemovalRequest } from "@/types";

interface TimelineEvent {
  id: string;
  type: "discovered" | "removal_submitted" | "removal_verified" | "reappeared";
  date: string;
  title: string;
  description: string;
  severity?: string;
  source?: string;
  category?: string;
}

export default function HistoryPage() {
  const router = useRouter();
  const [events, setEvents] = useState<TimelineEvent[]>([]);
  const [filteredEvents, setFilteredEvents] = useState<TimelineEvent[]>([]);
  const [loading, setLoading] = useState(true);
  const [severityFilter, setSeverityFilter] = useState<string>("all");
  const [typeFilter, setTypeFilter] = useState<string>("all");

  useEffect(() => {
    // No client-side token check: the session is an HttpOnly cookie the page cannot see.
    // If it is missing or expired the first API call answers 401 and the API client sends
    // the user to /login.
    loadHistory();
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  useEffect(() => {
    applyFilters();
  }, [severityFilter, typeFilter, events]); // eslint-disable-line react-hooks/exhaustive-deps

  async function loadHistory() {
    setLoading(true);
    try {
      const [exposuresData, removalsData] = await Promise.all([
        listExposures(),
        listRemovalRequests(),
      ]);

      const timelineEvents: TimelineEvent[] = [];

      // Add exposure discovery events
      exposuresData.exposures.forEach((exp: Exposure) => {
        timelineEvents.push({
          id: `discovered-${exp.id}`,
          type: "discovered",
          date: exp.discovered_at,
          title: `New ${exp.severity} exposure found`,
          description: `${exp.source_name} • ${exp.pii_summary}`,
          severity: exp.severity,
          source: exp.source_name,
          category: exp.pii_category,
        });

        // Add reappearance events if applicable
        if (!exp.is_active && exp.removed_at) {
          // Check if it reappeared (simplified - in production, check removal status)
          // For now, we'll just show removed status
        }
      });

      // Add removal events
      removalsData.requests.forEach((req: RemovalRequest) => {
        if (req.submitted_at) {
          timelineEvents.push({
            id: `submitted-${req.id}`,
            type: "removal_submitted",
            date: req.submitted_at,
            title: "Removal request submitted",
            description: `${req.source_name} • ${req.procedure_used || "web form"}`,
            source: req.source_name,
          });
        }

        if (req.verified_at && req.status === "verified_removed") {
          timelineEvents.push({
            id: `verified-${req.id}`,
            type: "removal_verified",
            date: req.verified_at,
            title: "Removal verified",
            description: `${req.source_name} • Information successfully removed`,
            source: req.source_name,
          });
        }

        if (req.status === "reappeared") {
          timelineEvents.push({
            id: `reappeared-${req.id}`,
            type: "reappeared",
            date: req.updated_at,
            title: "Information reappeared",
            description: `${req.source_name} • Previously removed data detected again`,
            source: req.source_name,
            severity: "high",
          });
        }
      });

      // Sort by date descending
      timelineEvents.sort(
        (a, b) => new Date(b.date).getTime() - new Date(a.date).getTime()
      );

      setEvents(timelineEvents);
      setFilteredEvents(timelineEvents);
    } catch (err) {
      console.error("Failed to load history:", err);
    } finally {
      setLoading(false);
    }
  }

  function applyFilters() {
    let filtered = events;

    if (severityFilter !== "all") {
      filtered = filtered.filter((e) => e.severity === severityFilter);
    }

    if (typeFilter !== "all") {
      filtered = filtered.filter((e) => e.type === typeFilter);
    }

    setFilteredEvents(filtered);
  }

  function handleExport() {
    // Export to CSV
    const csv = [
      "Date,Type,Title,Description,Severity,Source",
      ...filteredEvents.map((e) =>
        [
          new Date(e.date).toLocaleString(),
          e.type,
          e.title,
          e.description.replace(/,/g, ";"),
          e.severity || "",
          e.source || "",
        ].join(",")
      ),
    ].join("\n");

    const blob = new Blob([csv], { type: "text/csv" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = `vectorscrub-history-${new Date().toISOString().split("T")[0]}.csv`;
    a.click();
    URL.revokeObjectURL(url);
  }

  function getEventIcon(type: string) {
    switch (type) {
      case "discovered":
        return <AlertTriangle className="w-5 h-5 text-orange-400" />;
      case "removal_submitted":
        return <Send className="w-5 h-5 text-blue-400" />;
      case "removal_verified":
        return <CheckCircle className="w-5 h-5 text-green-400" />;
      case "reappeared":
        return <RefreshCcw className="w-5 h-5 text-red-400" />;
      default:
        return <Calendar className="w-5 h-5 text-gray-400" />;
    }
  }

  function getEventBorderColor(type: string) {
    switch (type) {
      case "discovered":
        return "border-l-orange-400";
      case "removal_submitted":
        return "border-l-blue-400";
      case "removal_verified":
        return "border-l-green-400";
      case "reappeared":
        return "border-l-red-400";
      default:
        return "border-l-gray-400";
    }
  }

  function handleSignOut() {
    void logout().finally(() => router.push("/"));
  }

  if (loading) {
    return (
      <div className="min-h-screen flex items-center justify-center">
        <div className="text-gray-400 text-sm animate-pulse">
          Loading exposure history…
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
            onClick={() => router.push("/dashboard/billing")}
            className="text-gray-400 hover:text-white text-sm font-medium transition-colors"
          >
            Billing
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
        {/* Header */}
        <div className="mb-8">
          <h1 className="text-2xl font-bold text-white mb-2">
            Exposure History Timeline
          </h1>
          <p className="text-gray-400 text-sm">
            A chronological record of all exposure-related events
          </p>
        </div>

        {/* Filters */}
        <div className="flex flex-col sm:flex-row gap-4 mb-8">
          <div className="flex items-center gap-2">
            <Filter className="w-4 h-4 text-gray-400" />
            <select
              value={typeFilter}
              onChange={(e) => setTypeFilter(e.target.value)}
              className="bg-gray-800 border border-gray-700 text-white text-sm rounded-lg px-3 py-2 focus:ring-2 focus:ring-teal-600 focus:outline-none"
            >
              <option value="all">All Events</option>
              <option value="discovered">Discovered</option>
              <option value="removal_submitted">Submitted</option>
              <option value="removal_verified">Verified</option>
              <option value="reappeared">Reappeared</option>
            </select>
          </div>
          <select
            value={severityFilter}
            onChange={(e) => setSeverityFilter(e.target.value)}
            className="bg-gray-800 border border-gray-700 text-white text-sm rounded-lg px-3 py-2 focus:ring-2 focus:ring-teal-600 focus:outline-none"
          >
            <option value="all">All Severities</option>
            <option value="critical">Critical</option>
            <option value="high">High</option>
            <option value="medium">Medium</option>
            <option value="low">Low</option>
          </select>
          <button
            onClick={handleExport}
            className="ml-auto flex items-center gap-2 bg-gray-800 hover:bg-gray-700 text-white px-4 py-2 rounded-lg text-sm font-medium transition-colors"
          >
            <Download className="w-4 h-4" />
            Export CSV
          </button>
        </div>

        {/* Timeline */}
        {filteredEvents.length === 0 ? (
          <div className="bg-gray-900 border border-gray-800 rounded-xl p-12 text-center">
            <Calendar className="w-10 h-10 text-gray-600 mx-auto mb-3" />
            <p className="text-gray-400 mb-1">No events found</p>
            <p className="text-gray-600 text-sm">
              Adjust your filters or run a scan to generate exposure events.
            </p>
          </div>
        ) : (
          <div className="space-y-4">
            {filteredEvents.map((event, index) => (
              <div
                key={event.id}
                className={`bg-gray-900 border border-gray-800 ${getEventBorderColor(
                  event.type
                )} border-l-4 rounded-xl p-5 flex gap-4`}
              >
                <div className="flex-shrink-0 mt-0.5">
                  {getEventIcon(event.type)}
                </div>
                <div className="flex-1 min-w-0">
                  <div className="flex items-start justify-between gap-4 mb-2">
                    <h3 className="text-white font-medium text-sm">
                      {event.title}
                    </h3>
                    <span className="text-xs text-gray-500 whitespace-nowrap">
                      {formatDate(event.date)}
                    </span>
                  </div>
                  <p className="text-gray-400 text-sm mb-2">
                    {event.description}
                  </p>
                  <div className="flex flex-wrap gap-2">
                    {event.severity && (
                      <span
                        className={`text-xs px-2 py-0.5 rounded-full border font-medium ${severityColor(
                          event.severity
                        )}`}
                      >
                        {event.severity}
                      </span>
                    )}
                    {event.category && (
                      <span className="text-xs bg-gray-800 text-gray-400 px-2 py-0.5 rounded-full">
                        {piiCategoryLabel(event.category)}
                      </span>
                    )}
                  </div>
                </div>
              </div>
            ))}
          </div>
        )}
      </main>
    </div>
  );
}
