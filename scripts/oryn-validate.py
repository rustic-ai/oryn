#!/usr/bin/env python3
"""Repository entry point for the consolidated validation package."""
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools/validation"))
from oryn_validation.cli import main

if __name__ == "__main__":
    raise SystemExit(main())
