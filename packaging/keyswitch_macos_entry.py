"""Build-only entry point for the native macOS application."""

from keyswitch.macos_app import main


if __name__ == "__main__":
    raise SystemExit(main())
