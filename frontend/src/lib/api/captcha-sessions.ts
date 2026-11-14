/**
 * CAPTCHA Session API Client - Phase 3
 *
 * Handles human-in-the-loop CAPTCHA intervention sessions.
 */

export interface CaptchaSession {
  id: string;
  customerId: string;
  removalRequestId: string;
  broker: string;
  status: 'waiting_for_customer' | 'customer_solving' | 'solved' | 'expired' | 'failed';
  sessionUrl: string;
  pageUrl: string;
  screenshotHash?: string;
  createdAt: string;
  expiresAt: string;
  solvedAt?: string;
  failedAt?: string;
  timeRemainingMinutes?: number;
}

export interface PendingSessionsResponse {
  sessions: CaptchaSession[];
  count: number;
}

/**
 * Get all pending CAPTCHA sessions for the authenticated customer.
 */
export async function getPendingCaptchaSessions(): Promise<PendingSessionsResponse> {
  const response = await fetch('/api/v1/captcha-sessions/pending', {
    method: 'GET',
    credentials: 'include',
    headers: {
      'Content-Type': 'application/json',
    },
  });

  if (!response.ok) {
    throw new Error(`Failed to fetch pending sessions: ${response.statusText}`);
  }

  return response.json();
}

/**
 * Get specific CAPTCHA session details.
 */
export async function getCaptchaSession(sessionId: string): Promise<CaptchaSession> {
  const response = await fetch(`/api/v1/captcha-sessions/${sessionId}`, {
    method: 'GET',
    credentials: 'include',
    headers: {
      'Content-Type': 'application/json',
    },
  });

  if (!response.ok) {
    if (response.status === 404) {
      throw new Error('Session not found');
    }
    if (response.status === 403) {
      throw new Error('You do not have permission to access this session');
    }
    throw new Error(`Failed to fetch session: ${response.statusText}`);
  }

  return response.json();
}

/**
 * Mark session as solved (customer completed CAPTCHA).
 */
export async function markSessionSolved(sessionId: string): Promise<{ success: boolean }> {
  const response = await fetch(`/api/v1/captcha-sessions/${sessionId}/solved`, {
    method: 'POST',
    credentials: 'include',
    headers: {
      'Content-Type': 'application/json',
    },
  });

  if (!response.ok) {
    throw new Error(`Failed to mark session as solved: ${response.statusText}`);
  }

  return response.json();
}

/**
 * Mark session as failed (customer couldn't solve CAPTCHA).
 */
export async function markSessionFailed(
  sessionId: string,
  reason?: string
): Promise<{ success: boolean }> {
  const response = await fetch(`/api/v1/captcha-sessions/${sessionId}/failed`, {
    method: 'POST',
    credentials: 'include',
    headers: {
      'Content-Type': 'application/json',
    },
    body: JSON.stringify({ reason }),
  });

  if (!response.ok) {
    throw new Error(`Failed to mark session as failed: ${response.statusText}`);
  }

  return response.json();
}

/**
 * Get screenshot evidence for a session.
 */
export function getSessionScreenshotUrl(sessionId: string): string {
  return `/api/v1/captcha-sessions/${sessionId}/screenshot`;
}
