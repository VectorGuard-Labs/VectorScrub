"use client";

import { useEffect, useState } from "react";
import {
  LineChart,
  Line,
  AreaChart,
  Area,
  XAxis,
  YAxis,
  CartesianGrid,
  Tooltip,
  ResponsiveContainer,
  Legend,
} from "recharts";
import { TrendingUp, TrendingDown, Minus } from "lucide-react";
import { getPrivacyScoreHistory } from "@/lib/api";
import type { PrivacyScoreTrend } from "@/lib/api";

export default function PrivacyScoreChart() {
  const [trend, setTrend] = useState<PrivacyScoreTrend | null>(null);
  const [timeRange, setTimeRange] = useState<number>(30);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    loadTrend();
  }, [timeRange]);

  async function loadTrend() {
    setLoading(true);
    try {
      const data = await getPrivacyScoreHistory(timeRange);
      setTrend(data);
    } catch (err) {
      console.error("Failed to load privacy score history:", err);
    } finally {
      setLoading(false);
    }
  }

  if (loading) {
    return (
      <div className="bg-gray-900 border border-gray-800 rounded-xl p-8">
        <div className="animate-pulse">
          <div className="h-4 bg-gray-800 rounded w-1/4 mb-4"></div>
          <div className="h-64 bg-gray-800 rounded"></div>
        </div>
      </div>
    );
  }

  if (!trend || trend.history.length === 0) {
    return (
      <div className="bg-gray-900 border border-gray-800 rounded-xl p-8 text-center">
        <p className="text-gray-400">
          Privacy score history will appear here after your first scan.
        </p>
      </div>
    );
  }

  // Format data for charts
  const scoreData = trend.history.map((entry) => ({
    date: new Date(entry.recorded_at).toLocaleDateString("en-US", {
      month: "short",
      day: "numeric",
    }),
    score: Math.round(entry.score),
    critical: entry.critical_count,
    high: entry.high_count,
    medium: entry.medium_count,
    low: entry.low_count,
  }));

  const delta = trend.score_delta ?? 0;
  const deltaIcon = delta > 0 ? TrendingUp : delta < 0 ? TrendingDown : Minus;
  const DeltaIcon = deltaIcon;
  const deltaColor =
    delta > 0
      ? "text-green-400"
      : delta < 0
        ? "text-red-400"
        : "text-gray-400";

  return (
    <div className="space-y-6">
      {/* Header with time range selector */}
      <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-4">
        <div>
          <h2 className="text-lg font-semibold text-white mb-1">
            Privacy Score Trend
          </h2>
          {trend.previous_score !== null && (
            <div className={`flex items-center gap-1.5 text-sm ${deltaColor}`}>
              <DeltaIcon className="w-4 h-4" />
              <span>
                {delta > 0 ? "+" : ""}
                {Math.round(delta)} points
              </span>
              <span className="text-gray-500">from {timeRange} days ago</span>
            </div>
          )}
        </div>
        <div className="flex gap-2">
          {[7, 30, 90, 365].map((days) => (
            <button
              key={days}
              onClick={() => setTimeRange(days)}
              className={`px-3 py-1.5 rounded-lg text-xs font-medium transition-colors ${
                timeRange === days
                  ? "bg-teal-600 text-white"
                  : "bg-gray-800 text-gray-400 hover:bg-gray-700"
              }`}
            >
              {days}d
            </button>
          ))}
        </div>
      </div>

      {/* Score trend line chart */}
      <div className="bg-gray-900 border border-gray-800 rounded-xl p-6">
        <h3 className="text-sm font-medium text-gray-400 mb-4">
          Privacy Score Over Time
        </h3>
        <ResponsiveContainer width="100%" height={200}>
          <LineChart data={scoreData}>
            <CartesianGrid strokeDasharray="3 3" stroke="#374151" />
            <XAxis
              dataKey="date"
              stroke="#9CA3AF"
              style={{ fontSize: "12px" }}
            />
            <YAxis
              domain={[0, 100]}
              stroke="#9CA3AF"
              style={{ fontSize: "12px" }}
            />
            <Tooltip
              contentStyle={{
                backgroundColor: "#1F2937",
                border: "1px solid #374151",
                borderRadius: "8px",
                fontSize: "12px",
              }}
              labelStyle={{ color: "#D1D5DB" }}
            />
            <Line
              type="monotone"
              dataKey="score"
              stroke="#14B8A6"
              strokeWidth={2}
              dot={{ fill: "#14B8A6", r: 4 }}
              activeDot={{ r: 6 }}
            />
          </LineChart>
        </ResponsiveContainer>
      </div>

      {/* Exposure breakdown stacked area chart */}
      <div className="bg-gray-900 border border-gray-800 rounded-xl p-6">
        <h3 className="text-sm font-medium text-gray-400 mb-4">
          Exposure Breakdown by Severity
        </h3>
        <ResponsiveContainer width="100%" height={200}>
          <AreaChart data={scoreData}>
            <CartesianGrid strokeDasharray="3 3" stroke="#374151" />
            <XAxis
              dataKey="date"
              stroke="#9CA3AF"
              style={{ fontSize: "12px" }}
            />
            <YAxis stroke="#9CA3AF" style={{ fontSize: "12px" }} />
            <Tooltip
              contentStyle={{
                backgroundColor: "#1F2937",
                border: "1px solid #374151",
                borderRadius: "8px",
                fontSize: "12px",
              }}
              labelStyle={{ color: "#D1D5DB" }}
            />
            <Legend
              wrapperStyle={{ fontSize: "12px" }}
              iconType="circle"
            />
            <Area
              type="monotone"
              dataKey="critical"
              stackId="1"
              stroke="#DC2626"
              fill="#DC2626"
              fillOpacity={0.8}
              name="Critical"
            />
            <Area
              type="monotone"
              dataKey="high"
              stackId="1"
              stroke="#F97316"
              fill="#F97316"
              fillOpacity={0.8}
              name="High"
            />
            <Area
              type="monotone"
              dataKey="medium"
              stackId="1"
              stroke="#FBBF24"
              fill="#FBBF24"
              fillOpacity={0.8}
              name="Medium"
            />
            <Area
              type="monotone"
              dataKey="low"
              stackId="1"
              stroke="#10B981"
              fill="#10B981"
              fillOpacity={0.8}
              name="Low"
            />
          </AreaChart>
        </ResponsiveContainer>
      </div>
    </div>
  );
}
