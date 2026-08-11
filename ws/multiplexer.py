"""Terminal multiplexer (tmux) and multi-window terminal launcher."""

import logging
import os
from pathlib import Path
import shutil
import subprocess
from typing import Sequence

logger = logging.getLogger("ws.multiplexer")


class TmuxLauncher:
    """Launches workspace services into tiled split panes inside a project-wide tmux session."""

    @classmethod
    def is_available(cls) -> bool:
        """Check if tmux executable is available on PATH."""
        return shutil.which("tmux") is not None

    @classmethod
    def session_name(cls, project_name: str) -> str:
        """Return project-wide session name."""
        return f"ws-{project_name}"

    @classmethod
    def is_session_running(cls, project_name: str) -> bool:
        """Check if project tmux session is active."""
        if not cls.is_available():
            return False
        res = subprocess.run(
            ["tmux", "has-session", "-t", cls.session_name(project_name)],
            capture_output=True,
            check=False,
        )
        return res.returncode == 0

    @classmethod
    def is_window_running(cls, project_name: str, workspace_name: str) -> bool:
        """Check if workspace window exists inside project tmux session."""
        if not cls.is_session_running(project_name):
            return False
        res = subprocess.run(
            ["tmux", "list-windows", "-t", cls.session_name(project_name), "-F", "#{window_name}"],
            capture_output=True,
            text=True,
            check=False,
        )
        return workspace_name in res.stdout.splitlines()

    @classmethod
    def list_panes(cls, project_name: str, workspace_name: str) -> list[str]:
        """List running pane titles/names for a workspace window."""
        if not cls.is_window_running(project_name, workspace_name):
            return []
        sess = cls.session_name(project_name)
        res = subprocess.run(
            ["tmux", "list-panes", "-t", f"{sess}:{workspace_name}", "-F", "#{pane_title}"],
            capture_output=True,
            text=True,
            check=False,
        )
        return [line.strip() for line in res.stdout.splitlines() if line.strip()]

    @classmethod
    def launch(
        cls,
        workspace_name: str,
        services: Sequence[tuple[str, str, str, dict[str, str]]],
        project_name: str,
    ) -> bool:
        """Launch services in a project-wide tmux session with a window named after the workspace."""
        if not cls.is_available() or not services:
            return False

        sess = cls.session_name(project_name)
        layout = "even-horizontal" if len(services) <= 3 else "tiled"

        if not cls.is_session_running(project_name):
            # Create new project session with the workspace as the first window
            first_svc = services[0]
            s_name, s_cwd, _, _ = first_svc
            bridge_cmd = f"ws bridge {workspace_name} {s_name}"
            subprocess.run(
                [
                    "tmux", "new-session", "-d",
                    "-s", sess,
                    "-n", workspace_name,
                    "-c", s_cwd,
                    bridge_cmd,
                ],
                check=True,
            )
            subprocess.run(["tmux", "select-pane", "-t", f"{sess}:{workspace_name}.0", "-T", s_name], check=False)

            for svc in services[1:]:
                s_name, s_cwd, _, _ = svc
                bridge_cmd = f"ws bridge {workspace_name} {s_name}"
                subprocess.run(
                    [
                        "tmux", "split-window",
                        "-h",
                        "-t", f"{sess}:{workspace_name}",
                        "-c", s_cwd,
                        bridge_cmd,
                    ],
                    check=True,
                )
                subprocess.run(["tmux", "select-pane", "-t", f"{sess}:{workspace_name}", "-T", s_name], check=False)
            subprocess.run(["tmux", "select-layout", "-t", f"{sess}:{workspace_name}", layout], check=False)

        elif not cls.is_window_running(project_name, workspace_name):
            # Session exists, create new window for this workspace
            first_svc = services[0]
            s_name, s_cwd, _, _ = first_svc
            bridge_cmd = f"ws bridge {workspace_name} {s_name}"
            subprocess.run(
                [
                    "tmux", "new-window",
                    "-t", sess,
                    "-n", workspace_name,
                    "-c", s_cwd,
                    bridge_cmd,
                ],
                check=True,
            )
            subprocess.run(["tmux", "select-pane", "-t", f"{sess}:{workspace_name}.0", "-T", s_name], check=False)

            for svc in services[1:]:
                s_name, s_cwd, _, _ = svc
                bridge_cmd = f"ws bridge {workspace_name} {s_name}"
                subprocess.run(
                    [
                        "tmux", "split-window",
                        "-h",
                        "-t", f"{sess}:{workspace_name}",
                        "-c", s_cwd,
                        bridge_cmd,
                    ],
                    check=True,
                )
                subprocess.run(["tmux", "select-pane", "-t", f"{sess}:{workspace_name}", "-T", s_name], check=False)
            subprocess.run(["tmux", "select-layout", "-t", f"{sess}:{workspace_name}", layout], check=False)

        else:
            # Window already running: add any missing service panes
            existing_panes = cls.list_panes(project_name, workspace_name)
            for svc in services:
                s_name, s_cwd, _, _ = svc
                if s_name not in existing_panes:
                    bridge_cmd = f"ws bridge {workspace_name} {s_name}"
                    subprocess.run(
                        [
                            "tmux", "split-window",
                            "-h",
                            "-t", f"{sess}:{workspace_name}",
                            "-c", s_cwd,
                            bridge_cmd,
                        ],
                        check=True,
                    )
                    subprocess.run(["tmux", "select-pane", "-t", f"{sess}:{workspace_name}", "-T", s_name], check=False)
                    subprocess.run(["tmux", "select-layout", "-t", f"{sess}:{workspace_name}", layout], check=False)



        # Select workspace window
        subprocess.run(["tmux", "select-window", "-t", f"{sess}:{workspace_name}"], check=False)

        # Attach / switch to window
        cls.attach(workspace_name=workspace_name, project_name=project_name, all_panes=True)
        return True


    @classmethod
    def attach(
        cls,
        workspace_name: str,
        project_name: str,
        repo_name: str | None = None,
        all_panes: bool = False,
    ) -> bool:
        """Attach to workspace window. If not all_panes, zoom the focused pane fullscreen."""
        if not cls.is_session_running(project_name):
            return False

        sess = cls.session_name(project_name)
        target_win = f"{sess}:{workspace_name}"

        # Switch to target window
        subprocess.run(["tmux", "select-window", "-t", target_win], check=False)

        # Check if target window is currently zoomed
        zoomed_check = subprocess.run(
            ["tmux", "list-windows", "-t", sess, "-F", "#{window_name}:#{window_zoomed_flag}"],
            capture_output=True,
            text=True,
            check=False,
        )
        is_zoomed = any(f"{workspace_name}:1" in line for line in zoomed_check.stdout.splitlines())

        if all_panes:
            # Whole workspace window: ensure unzoomed tiled view
            if is_zoomed:
                subprocess.run(["tmux", "resize-pane", "-t", target_win, "-Z"], check=False)
        else:
            # Single service / default: zoom pane fullscreen
            if not is_zoomed:
                subprocess.run(["tmux", "resize-pane", "-t", target_win, "-Z"], check=False)

        # Attach or switch-client if already inside tmux
        if os.environ.get("TMUX"):
            os.system(f"tmux switch-client -t {target_win}")
        else:
            os.system(f"tmux attach-session -t {target_win}")
        return True

    @classmethod
    def kill_workspace(cls, workspace_name: str, project_name: str) -> bool:
        """Kill workspace window in project tmux session."""
        if not cls.is_session_running(project_name):
            return False
        sess = cls.session_name(project_name)
        res = subprocess.run(
            ["tmux", "kill-window", "-t", f"{sess}:{workspace_name}"],
            capture_output=True,
            check=False,
        )
        # If no more windows remain, terminate session
        win_list = subprocess.run(
            ["tmux", "list-windows", "-t", sess],
            capture_output=True,
            text=True,
            check=False,
        )
        if not win_list.stdout.strip():
            cls.kill_session(project_name)
        return res.returncode == 0

    @classmethod
    def kill_session(cls, project_name: str) -> bool:
        """Kill entire project tmux session."""
        if not cls.is_available():
            return False
        sess = cls.session_name(project_name)
        res = subprocess.run(["tmux", "kill-session", "-t", sess], capture_output=True, check=False)
        return res.returncode == 0

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


class ZellijLauncher:
    """Launches workspace services into tiled split panes inside a project-wide Zellij session."""

    @classmethod
    def is_available(cls) -> bool:
        """Check if zellij executable is available on PATH."""
        return shutil.which("zellij") is not None

    @classmethod
    def session_name(cls, project_name: str) -> str:
        """Return project-wide session name."""
        return f"ws-{project_name}"

    @classmethod
    def is_session_running(cls, project_name: str) -> bool:
        """Check if project zellij session is active."""
        if not cls.is_available():
            return False
        try:
            res = subprocess.run(["zellij", "list-sessions"], capture_output=True, text=True, check=False)
            sess = cls.session_name(project_name)
            for line in res.stdout.splitlines():
                if sess in line and "EXITED" not in line:
                    return True
            return False
        except Exception:
            return False

    @classmethod
    def generate_layout_kdl(
        cls,
        workspace_name: str,
        services: Sequence[tuple[str, str, str, dict[str, str]]],
    ) -> str:
        """Generate a Zellij KDL layout file for the workspace services."""
        panes_kdl = []
        for s_name, s_cwd, _, _ in services:
            escaped_cwd = s_cwd.replace("\\", "\\\\").replace('"', '\\"')
            panes_kdl.append(
                f'        pane name="{s_name}" cwd="{escaped_cwd}" command="ws" {{\n'
                f'            args "bridge" "{workspace_name}" "{s_name}"\n'
                f'        }}'
            )


        n_services = len(services)
        if n_services <= 1:
            body = "\n".join(panes_kdl)
        elif n_services == 2:
            body = (
                '        pane split_direction="vertical" {\n'
                f'{panes_kdl[0]}\n'
                f'{panes_kdl[1]}\n'
                '        }'
            )
        elif n_services == 3:
            body = (
                '        pane split_direction="vertical" {\n'
                f'{panes_kdl[0]}\n'
                '            pane split_direction="horizontal" {\n'
                f'        {panes_kdl[1].strip()}\n'
                f'        {panes_kdl[2].strip()}\n'
                '            }\n'
                '        }'
            )
        else:
            left_panes = panes_kdl[::2]
            right_panes = panes_kdl[1::2]
            left_str = "\n".join([f"            {p.strip()}" for p in left_panes])
            right_str = "\n".join([f"            {p.strip()}" for p in right_panes])
            body = (
                '        pane split_direction="vertical" {\n'
                '            pane split_direction="horizontal" {\n'
                f'{left_str}\n'
                '            }\n'
                '            pane split_direction="horizontal" {\n'
                f'{right_str}\n'
                '            }\n'
                '        }'
            )

        kdl = (
            'layout {\n'
            '    default_tab_template {\n'
            '        children\n'
            '        pane size=1 borderless=true {\n'
            '            plugin location="zellij:compact-bar"\n'
            '        }\n'
            '    }\n'
            f'    tab name="{workspace_name}" {{\n'
            f'{body}\n'
            '    }\n'
            '}\n'
        )
        return kdl

    @classmethod
    def launch(
        cls,
        workspace_name: str,
        services: Sequence[tuple[str, str, str, dict[str, str]]],
        project_name: str,
        ws_dir: Path,
    ) -> bool:
        """Launch services in a project-wide zellij session with a tab for the workspace."""
        if not cls.is_available() or not services:
            return False

        sess = cls.session_name(project_name)
        layout_kdl = cls.generate_layout_kdl(workspace_name, services)
        layout_file = ws_dir / ".ws" / "zellij.kdl"
        layout_file.parent.mkdir(parents=True, exist_ok=True)
        layout_file.write_text(layout_kdl, encoding="utf-8")

        if not cls.is_session_running(project_name):
            # Create session with the workspace layout as default layout
            os.system(f"zellij attach -c {sess} options --default-layout {layout_file}")
        else:
            # Session already running: add new tab with layout, then attach
            subprocess.run(["zellij", "--session", sess, "--layout", str(layout_file)], check=False)
            if not os.environ.get("ZELLIJ"):
                os.system(f"zellij attach {sess}")
        return True

    @classmethod
    def attach(
        cls,
        workspace_name: str,
        project_name: str,
        repo_name: str | None = None,
        all_panes: bool = False,
    ) -> bool:
        """Attach to project zellij session and focus the workspace tab."""
        if not cls.is_available():
            return False
        sess = cls.session_name(project_name)
        if not cls.is_session_running(project_name):
            layout_file = Path.cwd() / "workspaces" / workspace_name / ".ws" / "zellij.kdl"
            if layout_file.exists():
                os.system(f"zellij attach -c {sess} options --default-layout {layout_file}")
            else:
                os.system(f"zellij attach -c {sess}")
        else:
            os.system(f"zellij attach {sess}")
        return True



    @classmethod
    def kill_workspace(cls, workspace_name: str, project_name: str) -> bool:
        """Kill workspace tab or session."""
        if not cls.is_available() or not cls.is_session_running(project_name):
            return False
        subprocess.run(["zellij", "action", "close-tab"], check=False)
        return True

    @classmethod
    def kill_session(cls, project_name: str) -> bool:
        """Kill entire project zellij session."""
        if not cls.is_available():
            return False
        sess = cls.session_name(project_name)
        res = subprocess.run(["zellij", "kill-session", sess], capture_output=True, check=False)
        return res.returncode == 0


