"""Build-only entry point for the native KeySwitch executable."""

from keyswitch.app import main


if __name__ == "__main__":
    raise SystemExit(main())
