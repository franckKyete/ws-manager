"""Terminal multiplexer (tmux) and multi-window terminal launcher."""

import logging
import os
from pathlib import Path
import shutil
import subprocess
from typing import Sequence

logger = logging.getLogger("ws.multiplexer")


class TmuxLauncher:
    """Launches workspace services into tiled split panes inside a tmux session."""

    @classmethod
    def is_available(cls) -> bool:
        """Check if tmux executable is available on PATH."""
        return shutil.which("tmux") is not None

    @classmethod
    def launch(
        cls,
        workspace_name: str,
        services: Sequence[tuple[str, str, str, dict[str, str]]],
    ) -> bool:
        """Launch services in a tmux session named ws-<workspace_name> with tiled split panes.

        services is list of (service_name, worktree_path_str, launch_command, env_vars).
        """
        if not cls.is_available():
            return False

        session_name = f"ws-{workspace_name}"

        # Kill existing session with same name if already running
        subprocess.run(f"tmux kill-session -t {session_name}", shell=True, capture_output=True, check=False)

        first_svc = services[0]
        s_name, s_cwd, s_cmd, s_env = first_svc

        # Create initial tmux window
        cmd_with_env = cls._build_env_command(s_cmd, s_env, workspace_name, s_name)
        subprocess.run(
            ["tmux", "new-session", "-d", "-s", session_name, "-c", s_cwd, cmd_with_env],
            check=True,
        )

        # Split window for remaining services
        for idx, svc in enumerate(services[1:], start=1):
            s_name, s_cwd, s_cmd, s_env = svc
            cmd_with_env = cls._build_env_command(s_cmd, s_env, workspace_name, s_name)
            subprocess.run(
                ["tmux", "split-window", "-t", session_name, "-c", s_cwd, cmd_with_env],
                check=True,
            )
            # Rebalance layout tiled
            subprocess.run(["tmux", "select-layout", "-t", session_name, "tiled"], check=False)

        # Attach to the session in the current terminal
        os.system(f"tmux attach-session -t {session_name}")
        return True

    @staticmethod
    def _build_env_command(cmd: str, env: dict[str, str], ws_name: str, repo_name: str) -> str:
        """Build shell command with inline environment exports."""
        exports = " ".join([f"{k}={v}" for k, v in env.items() if "\n" not in str(v)])
        return f"export {exports} WORKSPACE_NAME={ws_name} REPO_NAME={repo_name} && {cmd}"


class TerminalLauncher:
    """Launches workspace services into separate native terminal tabs or windows."""

    SUPPORTED_TERMINALS = [
        "wezterm",
        "gnome-terminal",
        "kitty",
        "alacritty",
        "konsole",
        "xterm",
    ]

    @classmethod
    def detect_terminal(cls) -> str | None:
        """Detect available terminal emulator executable."""
        for term in cls.SUPPORTED_TERMINALS:
            if shutil.which(term):
                return term
        return None

    @classmethod
    def launch(
        cls,
        workspace_name: str,
        services: Sequence[tuple[str, str, str, dict[str, str]]],
    ) -> bool:
        """Open separate terminal windows/tabs for each service."""
        term = cls.detect_terminal()
        if not term:
            return False

        for svc in services:
            s_name, s_cwd, s_cmd, s_env = svc
            cmd_with_env = TmuxLauncher._build_env_command(s_cmd, s_env, workspace_name, s_name)

            if term == "wezterm":
                subprocess.Popen(["wezterm", "start", "--cwd", s_cwd, "bash", "-c", f"{cmd_with_env}; exec bash"])
            elif term == "gnome-terminal":
                subprocess.Popen(["gnome-terminal", "--tab", f"--title={s_name}", f"--working-directory={s_cwd}", "--", "bash", "-c", f"{cmd_with_env}; exec bash"])
            elif term == "kitty":
                subprocess.Popen(["kitty", "--directory", s_cwd, "--title", s_name, "bash", "-c", f"{cmd_with_env}; exec bash"])
            elif term == "alacritty":
                subprocess.Popen(["alacritty", "--working-directory", s_cwd, "-e", "bash", "-c", f"{cmd_with_env}; exec bash"])
            elif term == "konsole":
                subprocess.Popen(["konsole", "--workdir", s_cwd, "-e", "bash", "-c", f"{cmd_with_env}; exec bash"])
            elif term == "xterm":
                subprocess.Popen(["xterm", "-T", s_name, "-e", f"cd {s_cwd} && {cmd_with_env}; exec bash"])

        return True
