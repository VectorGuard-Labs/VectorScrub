#!/bin/bash
#
# Nightly Broker Health Check
#
# Runs automated health checks on all broker opt-out pages.
# Add to crontab to run nightly at 2 AM:
#
#   0 2 * * * /path/to/vectorscrub/scripts/health-check-nightly.sh
#

set -euo pipefail

# Configuration
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
AGENTS_DIR="$PROJECT_ROOT/agents"
LOG_DIR="$PROJECT_ROOT/logs"
LOG_FILE="$LOG_DIR/health-check-$(date +%Y%m%d).log"

# Create log directory if needed
mkdir -p "$LOG_DIR"

# Log start
echo "========================================" >> "$LOG_FILE"
echo "Broker Health Check - $(date)" >> "$LOG_FILE"
echo "========================================" >> "$LOG_FILE"

# Activate virtual environment if it exists
if [ -d "$PROJECT_ROOT/venv" ]; then
    source "$PROJECT_ROOT/venv/bin/activate"
fi

# Run health check
cd "$AGENTS_DIR"
python -m monitoring.health_checker >> "$LOG_FILE" 2>&1

# Check exit code
EXIT_CODE=$?
if [ $EXIT_CODE -eq 0 ]; then
    echo "✓ Health check completed successfully" >> "$LOG_FILE"
else
    echo "✗ Health check failed with code $EXIT_CODE" >> "$LOG_FILE"

    # Send alert (optional - configure based on your alerting system)
    # /usr/local/bin/send-alert "VectorScrub health check failed"
fi

echo "" >> "$LOG_FILE"

# Cleanup old logs (keep last 30 days)
find "$LOG_DIR" -name "health-check-*.log" -mtime +30 -delete

exit $EXIT_CODE
