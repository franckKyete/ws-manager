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


def parse_duration(val: str | int | float | None) -> int:
    """Parse duration value into seconds integer.

    Supported formats:
    - Integer / float: treated directly as seconds (e.g. 300 -> 300)
    - String with unit: '30s', '5m', '15m', '1h', '2d'
    - 'never', '0', 'none', '', None: returns 0 (disabled)
    """
    if val is None:
        return 0
    if isinstance(val, (int, float)):
        return max(0, int(val))

    s = str(val).strip().lower()
    if not s or s in ("never", "none", "false", "off", "0", "0s", "0m", "0h"):
        return 0

    m = re.match(r"^(\d+(?:\.\d+)?)\s*([a-z]+)?$", s)
    if not m:
        try:
            return max(0, int(float(s)))
        except ValueError:
            return 0

    amount = float(m.group(1))
    unit = m.group(2) or "s"

    if unit in ("s", "sec", "second", "seconds"):
        return max(0, int(amount))
    elif unit in ("m", "min", "minute", "minutes"):
        return max(0, int(amount * 60))
    elif unit in ("h", "hr", "hour", "hours"):
        return max(0, int(amount * 3600))
    elif unit in ("d", "day", "days"):
        return max(0, int(amount * 86400))
    elif unit in ("w", "week", "weeks"):
        return max(0, int(amount * 604800))
    return max(0, int(amount))


def format_duration(seconds: int) -> str:
    """Convert duration in seconds into human-readable shorthand (e.g. '5m', '1h')."""
    if seconds <= 0:
        return "0s"
    if seconds % 86400 == 0:
        return f"{seconds // 86400}d"
    if seconds % 3600 == 0:
        return f"{seconds // 3600}h"
    if seconds % 60 == 0:
        return f"{seconds // 60}m"
    return f"{seconds}s"


