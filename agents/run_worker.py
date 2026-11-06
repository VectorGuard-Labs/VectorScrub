"""
Entry point for the VectorScrub agent worker.
Run from the VectorScrub project root:

    python3 agents/run_worker.py

Or from inside the agents/ directory:

    python3 run_worker.py
"""
import asyncio
import sys
import os

# Ensure the agents/ directory is on the path so relative imports resolve
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from orchestrator.orchestrator import main

if __name__ == "__main__":
    asyncio.run(main())
