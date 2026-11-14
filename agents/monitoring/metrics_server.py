"""
Metrics HTTP server for VectorScrub agents.

Exposes Prometheus-compatible metrics endpoint and health dashboard.
Run this as a separate service alongside the agent workers.
"""
import asyncio
from typing import Dict, Any

from aiohttp import web
import structlog

from shared.metrics import get_metrics_collector, OperationType

log = structlog.get_logger(__name__)


async def metrics_handler(request: web.Request) -> web.Response:
    """
    Prometheus metrics endpoint.

    GET /metrics
    """
    try:
        collector = get_metrics_collector()
        metrics_text = await collector.export_prometheus()
        return web.Response(
            text=metrics_text,
            content_type="text/plain; version=0.0.4",
        )
    except Exception as exc:
        log.error("metrics_export_failed", error_type=type(exc).__name__)
        return web.Response(
            status=500,
            text=f"Metrics export failed: {type(exc).__name__}",
        )


async def health_dashboard_handler(request: web.Request) -> web.Response:
    """
    Human-readable health dashboard.

    GET /health
    """
    try:
        collector = get_metrics_collector()

        # Get metrics for each operation type
        discovery_health = await collector.get_all_broker_health(OperationType.DISCOVERY)
        removal_health = await collector.get_all_broker_health(OperationType.REMOVAL)
        smtp_metrics = await collector.get_smtp_metrics()

        # Build HTML response
        html = """
        <!DOCTYPE html>
        <html>
        <head>
            <title>VectorScrub Agent Metrics</title>
            <style>
                body { font-family: Arial, sans-serif; margin: 20px; background: #f5f5f5; }
                h1 { color: #333; }
                h2 { color: #666; margin-top: 30px; }
                table { border-collapse: collapse; width: 100%; background: white; margin-bottom: 20px; }
                th, td { text-align: left; padding: 12px; border: 1px solid #ddd; }
                th { background-color: #0066cc; color: white; }
                tr:nth-child(even) { background-color: #f9f9f9; }
                .success { color: green; font-weight: bold; }
                .warning { color: orange; font-weight: bold; }
                .error { color: red; font-weight: bold; }
                .metric-box { background: white; padding: 15px; margin-bottom: 15px; border-radius: 5px; }
            </style>
        </head>
        <body>
            <h1>VectorScrub Agent Metrics Dashboard</h1>
        """

        # Discovery metrics
        html += "<h2>Discovery Agent Health</h2>"
        if discovery_health:
            html += """
            <table>
                <tr>
                    <th>Broker</th>
                    <th>Total Requests</th>
                    <th>Success Rate</th>
                    <th>Avg Response (ms)</th>
                    <th>Failed</th>
                    <th>Rate Limited</th>
                    <th>Blocked</th>
                </tr>
            """
            for health in discovery_health[:20]:  # Top 20
                success_class = "success" if health.success_rate >= 80 else "warning" if health.success_rate >= 50 else "error"
                html += f"""
                <tr>
                    <td>{health.broker}</td>
                    <td>{health.total_requests}</td>
                    <td class="{success_class}">{health.success_rate:.1f}%</td>
                    <td>{health.avg_response_time_ms:.0f}</td>
                    <td>{health.failed_requests}</td>
                    <td>{health.rate_limited_count}</td>
                    <td>{health.blocked_count}</td>
                </tr>
                """
            html += "</table>"
        else:
            html += "<p>No discovery metrics available yet.</p>"

        # Removal metrics
        html += "<h2>Removal Agent Health</h2>"
        if removal_health:
            html += """
            <table>
                <tr>
                    <th>Broker</th>
                    <th>Total Attempts</th>
                    <th>Success Rate</th>
                    <th>Avg Duration (s)</th>
                    <th>Failed</th>
                    <th>CAPTCHA Blocked</th>
                </tr>
            """
            for health in removal_health[:20]:  # Top 20
                success_class = "success" if health.success_rate >= 70 else "warning" if health.success_rate >= 40 else "error"
                avg_duration = health.avg_response_time_ms / 1000 if health.avg_response_time_ms else 0
                html += f"""
                <tr>
                    <td>{health.broker}</td>
                    <td>{health.total_requests}</td>
                    <td class="{success_class}">{health.success_rate:.1f}%</td>
                    <td>{avg_duration:.1f}</td>
                    <td>{health.failed_requests}</td>
                    <td>{health.captcha_count}</td>
                </tr>
                """
            html += "</table>"
        else:
            html += "<p>No removal metrics available yet.</p>"

        # SMTP metrics
        html += "<h2>SMTP Email Delivery</h2>"
        html += '<div class="metric-box">'
        smtp_success_rate = (smtp_metrics.successful / smtp_metrics.total_sent * 100) if smtp_metrics.total_sent > 0 else 0
        smtp_class = "success" if smtp_success_rate >= 95 else "warning" if smtp_success_rate >= 80 else "error"
        html += f"""
        <p><strong>Total Sent:</strong> {smtp_metrics.total_sent}</p>
        <p><strong>Success Rate:</strong> <span class="{smtp_class}">{smtp_success_rate:.1f}%</span></p>
        <p><strong>Successful:</strong> {smtp_metrics.successful}</p>
        <p><strong>Failed:</strong> {smtp_metrics.failed}</p>
        <p><strong>Auth Failures:</strong> {smtp_metrics.auth_failures}</p>
        <p><strong>Connection Failures:</strong> {smtp_metrics.connection_failures}</p>
        <p><strong>Timeout Failures:</strong> {smtp_metrics.timeout_failures}</p>
        """
        html += "</div>"

        html += """
        <p style="margin-top: 40px; color: #666; font-size: 12px;">
            Auto-refreshes every 30 seconds.
            <a href="/metrics">View Prometheus metrics</a>
        </p>
        <script>setTimeout(function(){ location.reload(); }, 30000);</script>
        </body>
        </html>
        """

        return web.Response(text=html, content_type="text/html")

    except Exception as exc:
        log.error("health_dashboard_failed", error_type=type(exc).__name__)
        return web.Response(
            status=500,
            text=f"<html><body><h1>Error</h1><p>{type(exc).__name__}</p></body></html>",
            content_type="text/html",
        )


async def ping_handler(request: web.Request) -> web.Response:
    """Simple health check endpoint."""
    return web.Response(text="OK")


def create_app() -> web.Application:
    """Create the metrics server application."""
    app = web.Application()
    app.router.add_get("/metrics", metrics_handler)
    app.router.add_get("/health", health_dashboard_handler)
    app.router.add_get("/ping", ping_handler)
    return app


def main():
    """Run the metrics server."""
    import os

    port = int(os.environ.get("METRICS_PORT", "9090"))
    log.info("starting_metrics_server", port=port)

    app = create_app()
    web.run_app(app, host="0.0.0.0", port=port)


if __name__ == "__main__":
    main()
