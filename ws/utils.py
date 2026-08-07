"""Utility functions for time formatting and filesystem operations."""

from datetime import datetime, timezone
from pathlib import Path
import re


def get_iso_timestamp() -> str:
    """Return current UTC timestamp in ISO 8601 format."""
    return datetime.now(timezone.utc).replace(microsecond=0).isoformat()


def parse_timestamp(ts_str: str) -> datetime:
    """Parse string timestamp into datetime object."""
    ts_str = ts_str.strip()
    try:
        return datetime.fromisoformat(ts_str)
    except ValueError:
        pass

    # Fallback formats
    formats = [
        "%Y-%m-%dT%H:%M:%S",
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%d",
    ]
    for fmt in formats:
        try:
            return datetime.strptime(ts_str, fmt).replace(tzinfo=timezone.utc)
        except ValueError:
            continue

    raise ValueError(f"Cannot parse timestamp: '{ts_str}'")


def format_relative_time(timestamp_str: str) -> str:
    """Convert ISO timestamp string to human-readable relative time string.

    Examples:
    - just now
    - 5m ago
    - 2h ago
    - yesterday
    - 3 days ago
    """
    try:
        dt = parse_timestamp(timestamp_str)
    except Exception:
        return timestamp_str

    if dt.tzinfo is None:
        dt = dt.replace(tzinfo=timezone.utc)

    now = datetime.now(timezone.utc)
    diff = now - dt

    seconds = int(diff.total_seconds())
    if seconds < 0:
        return "just now"

    if seconds < 60:
        return "just now"
    elif seconds < 3600:
        minutes = seconds // 60
        return f"{minutes}m ago"
    elif seconds < 86400:
        hours = seconds // 3600
        return f"{hours}h ago"
    elif seconds < 172800:
        return "yesterday"
    else:
        days = seconds // 86400
        return f"{days} days ago"


def ensure_directory(path: Path) -> Path:
    """Ensure directory exists and return resolved path."""
    path.mkdir(parents=True, exist_ok=True)
    return path
