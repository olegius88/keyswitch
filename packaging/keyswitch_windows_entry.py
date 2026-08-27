"""Build-only entry point for the native Windows executable."""

from keyswitch.windows_app import main


if __name__ == "__main__":
    raise SystemExit(main())
