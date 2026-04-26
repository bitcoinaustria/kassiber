"""PyInstaller entry point for prerelease CLI binaries."""

from kassiber.cli.main import main


if __name__ == "__main__":
    raise SystemExit(main())
