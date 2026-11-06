"use client";

import { useState } from "react";
import { useRouter } from "next/navigation";
import { User, MapPin, Phone, Mail, Calendar, Plus, X } from "lucide-react";

export default function ProfilePage() {
  const router = useRouter();
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");
  const [success, setSuccess] = useState(false);

  const [formData, setFormData] = useState({
    full_name: "",
    street_address: "",
    apt_suite: "",
    city: "",
    state: "",
    zip_code: "",
    previous_addresses: [""],
    phone_numbers: [""],
    email_addresses: [""],
    date_of_birth: "",
  });

  const addField = (field: "previous_addresses" | "phone_numbers" | "email_addresses") => {
    setFormData({
      ...formData,
      [field]: [...formData[field], ""],
    });
  };

  const removeField = (field: "previous_addresses" | "phone_numbers" | "email_addresses", index: number) => {
    const updated = formData[field].filter((_, i) => i !== index);
    setFormData({
      ...formData,
      [field]: updated.length > 0 ? updated : [""],
    });
  };

  const updateField = (field: "previous_addresses" | "phone_numbers" | "email_addresses", index: number, value: string) => {
    const updated = [...formData[field]];
    updated[index] = value;
    setFormData({
      ...formData,
      [field]: updated,
    });
  };

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setLoading(true);
    setError("");
    setSuccess(false);

    // Build current address from components
    const current_address = [
      formData.street_address,
      formData.apt_suite ? `Apt ${formData.apt_suite}` : "",
      formData.city,
      formData.state,
      formData.zip_code,
    ]
      .filter((s) => s.trim() !== "")
      .join(", ");

    const payload = {
      full_name: formData.full_name,
      current_address,
      previous_addresses: formData.previous_addresses.filter((a) => a.trim() !== ""),
      phone_numbers: formData.phone_numbers.filter((p) => p.trim() !== ""),
      email_addresses: formData.email_addresses.filter((e) => e.trim() !== ""),
      date_of_birth: formData.date_of_birth || null,
    };

    try {
      const token = localStorage.getItem("vs_token");
      if (!token) {
        throw new Error("Not authenticated. Please log in again.");
      }

      const response = await fetch("/api/v1/me/identity", {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          Authorization: `Bearer ${token}`,
        },
        body: JSON.stringify(payload),
      });

      if (!response.ok) {
        let errorMsg = "Failed to save identity profile";
        try {
          const data = await response.json();
          errorMsg = data.message || errorMsg;
        } catch {
          errorMsg = `Server error: ${response.status} ${response.statusText}`;
        }
        throw new Error(errorMsg);
      }

      setSuccess(true);
      setTimeout(() => router.push("/dashboard"), 2000);
    } catch (err: any) {
      setError(err.message);
    } finally {
      setLoading(false);
    }
  };

  return (
    <div className="min-h-screen bg-gray-950 text-gray-100">
      <div className="max-w-3xl mx-auto px-6 py-12">
        <div className="mb-8">
          <h1 className="text-3xl font-bold text-white mb-2">Identity Profile</h1>
          <p className="text-gray-400">
            This information is used to scan data brokers and people-search sites.
            All data is encrypted with AES-256-GCM before storage.
          </p>
        </div>

        {success && (
          <div className="mb-6 p-4 bg-teal-950 border border-teal-800 rounded-lg text-teal-400">
            Profile saved successfully! Redirecting to dashboard...
          </div>
        )}

        {error && (
          <div className="mb-6 p-4 bg-red-950 border border-red-800 rounded-lg text-red-400">
            {error}
          </div>
        )}

        <form onSubmit={handleSubmit} className="space-y-6">
          {/* Full Name */}
          <div>
            <label className="flex items-center gap-2 text-sm font-medium text-gray-300 mb-2">
              <User className="w-4 h-4" />
              Full Name *
            </label>
            <input
              type="text"
              required
              value={formData.full_name}
              onChange={(e) => setFormData({ ...formData, full_name: e.target.value })}
              className="w-full px-4 py-2 bg-gray-900 border border-gray-700 rounded-lg text-white focus:outline-none focus:border-teal-500"
              placeholder="John Doe"
            />
          </div>

          {/* Current Address */}
          <div>
            <label className="flex items-center gap-2 text-sm font-medium text-gray-300 mb-2">
              <MapPin className="w-4 h-4" />
              Current Address *
            </label>
            <div className="space-y-2">
              <input
                type="text"
                required
                value={formData.street_address}
                onChange={(e) => setFormData({ ...formData, street_address: e.target.value })}
                className="w-full px-4 py-2 bg-gray-900 border border-gray-700 rounded-lg text-white focus:outline-none focus:border-teal-500"
                placeholder="Street address"
              />
              <input
                type="text"
                value={formData.apt_suite}
                onChange={(e) => setFormData({ ...formData, apt_suite: e.target.value })}
                className="w-full px-4 py-2 bg-gray-900 border border-gray-700 rounded-lg text-white focus:outline-none focus:border-teal-500"
                placeholder="Apt, suite, unit, etc. (optional)"
              />
              <div className="grid grid-cols-2 gap-2">
                <input
                  type="text"
                  required
                  value={formData.city}
                  onChange={(e) => setFormData({ ...formData, city: e.target.value })}
                  className="px-4 py-2 bg-gray-900 border border-gray-700 rounded-lg text-white focus:outline-none focus:border-teal-500"
                  placeholder="City"
                />
                <div className="grid grid-cols-2 gap-2">
                  <input
                    type="text"
                    required
                    maxLength={2}
                    value={formData.state}
                    onChange={(e) => setFormData({ ...formData, state: e.target.value.toUpperCase() })}
                    className="px-4 py-2 bg-gray-900 border border-gray-700 rounded-lg text-white focus:outline-none focus:border-teal-500"
                    placeholder="State"
                  />
                  <input
                    type="text"
                    required
                    maxLength={10}
                    value={formData.zip_code}
                    onChange={(e) => setFormData({ ...formData, zip_code: e.target.value })}
                    className="px-4 py-2 bg-gray-900 border border-gray-700 rounded-lg text-white focus:outline-none focus:border-teal-500"
                    placeholder="ZIP"
                  />
                </div>
              </div>
            </div>
          </div>

          {/* Previous Addresses */}
          <div>
            <label className="flex items-center gap-2 text-sm font-medium text-gray-300 mb-2">
              <MapPin className="w-4 h-4" />
              Previous Addresses
            </label>
            {formData.previous_addresses.map((addr, idx) => (
              <div key={idx} className="flex gap-2 mb-2">
                <input
                  type="text"
                  value={addr}
                  onChange={(e) => updateField("previous_addresses", idx, e.target.value)}
                  className="flex-1 px-4 py-2 bg-gray-900 border border-gray-700 rounded-lg text-white focus:outline-none focus:border-teal-500"
                  placeholder="Previous address"
                />
                {formData.previous_addresses.length > 1 && (
                  <button
                    type="button"
                    onClick={() => removeField("previous_addresses", idx)}
                    className="px-3 py-2 bg-gray-800 border border-gray-700 rounded-lg text-gray-400 hover:text-red-400 hover:border-red-700"
                  >
                    <X className="w-4 h-4" />
                  </button>
                )}
              </div>
            ))}
            <button
              type="button"
              onClick={() => addField("previous_addresses")}
              className="flex items-center gap-2 text-sm text-teal-400 hover:text-teal-300"
            >
              <Plus className="w-4 h-4" />
              Add another address
            </button>
          </div>

          {/* Phone Numbers */}
          <div>
            <label className="flex items-center gap-2 text-sm font-medium text-gray-300 mb-2">
              <Phone className="w-4 h-4" />
              Phone Numbers
            </label>
            {formData.phone_numbers.map((phone, idx) => (
              <div key={idx} className="flex gap-2 mb-2">
                <input
                  type="tel"
                  value={phone}
                  onChange={(e) => updateField("phone_numbers", idx, e.target.value)}
                  className="flex-1 px-4 py-2 bg-gray-900 border border-gray-700 rounded-lg text-white focus:outline-none focus:border-teal-500"
                  placeholder="(555) 123-4567"
                />
                {formData.phone_numbers.length > 1 && (
                  <button
                    type="button"
                    onClick={() => removeField("phone_numbers", idx)}
                    className="px-3 py-2 bg-gray-800 border border-gray-700 rounded-lg text-gray-400 hover:text-red-400 hover:border-red-700"
                  >
                    <X className="w-4 h-4" />
                  </button>
                )}
              </div>
            ))}
            <button
              type="button"
              onClick={() => addField("phone_numbers")}
              className="flex items-center gap-2 text-sm text-teal-400 hover:text-teal-300"
            >
              <Plus className="w-4 h-4" />
              Add another phone
            </button>
          </div>

          {/* Email Addresses */}
          <div>
            <label className="flex items-center gap-2 text-sm font-medium text-gray-300 mb-2">
              <Mail className="w-4 h-4" />
              Email Addresses *
            </label>
            {formData.email_addresses.map((email, idx) => (
              <div key={idx} className="flex gap-2 mb-2">
                <input
                  type="email"
                  value={email}
                  onChange={(e) => updateField("email_addresses", idx, e.target.value)}
                  className="flex-1 px-4 py-2 bg-gray-900 border border-gray-700 rounded-lg text-white focus:outline-none focus:border-teal-500"
                  placeholder="you@example.com"
                  required={idx === 0}
                />
                {formData.email_addresses.length > 1 && (
                  <button
                    type="button"
                    onClick={() => removeField("email_addresses", idx)}
                    className="px-3 py-2 bg-gray-800 border border-gray-700 rounded-lg text-gray-400 hover:text-red-400 hover:border-red-700"
                  >
                    <X className="w-4 h-4" />
                  </button>
                )}
              </div>
            ))}
            <button
              type="button"
              onClick={() => addField("email_addresses")}
              className="flex items-center gap-2 text-sm text-teal-400 hover:text-teal-300"
            >
              <Plus className="w-4 h-4" />
              Add another email
            </button>
          </div>

          {/* Date of Birth */}
          <div>
            <label className="flex items-center gap-2 text-sm font-medium text-gray-300 mb-2">
              <Calendar className="w-4 h-4" />
              Date of Birth (Optional)
            </label>
            <input
              type="date"
              value={formData.date_of_birth}
              onChange={(e) => setFormData({ ...formData, date_of_birth: e.target.value })}
              className="w-full px-4 py-2 bg-gray-900 border border-gray-700 rounded-lg text-white focus:outline-none focus:border-teal-500"
            />
          </div>

          {/* Submit */}
          <div className="flex gap-4 pt-4">
            <button
              type="submit"
              disabled={loading}
              className="flex-1 bg-teal-600 hover:bg-teal-500 text-white px-6 py-3 rounded-lg font-medium transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
            >
              {loading ? "Saving..." : "Save Identity Profile"}
            </button>
            <button
              type="button"
              onClick={() => router.push("/dashboard")}
              className="px-6 py-3 border border-gray-700 hover:border-gray-500 text-gray-300 rounded-lg font-medium transition-colors"
            >
              Cancel
            </button>
          </div>
        </form>

        <div className="mt-8 p-4 bg-gray-900/50 border border-gray-800 rounded-lg">
          <h3 className="text-sm font-semibold text-gray-300 mb-2">🔒 Privacy & Security</h3>
          <ul className="text-xs text-gray-500 space-y-1">
            <li>• All PII is encrypted with AES-256-GCM before database storage</li>
            <li>• Decryption only occurs server-side during active scans</li>
            <li>• Your data is never sold, shared, or used to train AI models</li>
            <li>• Master encryption key is stored separately from the database</li>
          </ul>
        </div>
      </div>
    </div>
  );
}
