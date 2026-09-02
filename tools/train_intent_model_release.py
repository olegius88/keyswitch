#!/usr/bin/env python3
"""Stable release entry point for the sealed intent-model trainer.

The v15 trainer normalizes metadata at the KSLM serialization boundary itself,
so release execution no longer needs to monkey-patch module globals.  Keeping a
small dedicated command makes the documented production invocation stable.
"""

from __future__ import annotations

import train_intent_model as trainer


def main() -> int:
    """Delegate to the trainer without changing process-global state."""

    return trainer.main()


if __name__ == "__main__":
    raise SystemExit(main())
