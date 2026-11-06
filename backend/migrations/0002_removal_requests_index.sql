-- Phase 2: Performance optimization for removal request polling
-- Composite index for orchestrator query: WHERE status = 'authorized' ORDER BY created_at ASC

CREATE INDEX idx_removal_requests_pending
ON removal_requests(status, created_at)
WHERE status = 'authorized';
