"""
Entry point for the VectorScrub agent worker.
Run from inside the agents/ directory:

    python3 run_worker.py

Do NOT run it from the repo root: settings load ".env" from the current directory, so the
root .env (which holds backend-only secrets) would be read and the agents refuse to start.
"""
import asyncio
import sys
import os

# Ensure the agents/ directory is on the path so relative imports resolve
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from orchestrator.orchestrator import main

if __name__ == "__main__":
    asyncio.run(main())
