"""Interactive Multi-Pane Terminal User Interface (TUI) for workspace service monitoring."""

from collections import deque
import logging
import os
from pathlib import Path
import re
import select
import sys
import threading
import time
from typing import Sequence

from rich import box
from rich.console import Console, RenderableType
from rich.live import Live
from rich.panel import Panel
from rich.table import Table
from rich.text import Text

from ws.process import ManagedService, ProcessSupervisor

logger = logging.getLogger("ws.tui")

# Strip destructive terminal control sequences (cursor repositioning, screen clear, mode switches)
# while preserving SGR color/style sequences (\x1b[...m)
NON_COLOR_ANSI_REGEX = re.compile(
    r"\x1b\[[0-9;?]*[A-LN-Za-ln-z]|\x1b\([AB012]|\x1b\][^\x07\x1b]*[\x07\x1b\\]|[\x00-\x08\x0b\x0c\x0e\x0f]"
)


def sanitize_terminal_line(line: str) -> str:
    """Sanitize raw ANSI line for embedded display in a TUI pane."""
    if not line:
        return ""
    # If line has carriage return (\r), take the latest segment
    if "\r" in line:
        parts = line.split("\r")
        line = parts[-1] if parts[-1].strip() else parts[0]
    # Remove terminal clear/cursor movement codes
    cleaned = NON_COLOR_ANSI_REGEX.sub("", line)
    return cleaned.rstrip("\r\n")


class WorkspaceTUI:
    """Rich interactive multi-pane terminal interface for concurrent workspace services."""

    def __init__(
        self,
        workspace_name: str,
        supervisor: ProcessSupervisor,
        initial_service: str | None = None,
        console: Console | None = None,
    ) -> None:
        self.workspace_name = workspace_name
        self.supervisor = supervisor
        self.console = console or Console()
        self.service_names = list(supervisor.services.keys())
        self.focused_index = 0
        if initial_service and initial_service in self.service_names:
            self.focused_index = self.service_names.index(initial_service)

        self.fullscreen_mode: bool = False
        self.interactive_mode: bool = False
        self.scroll_offsets: dict[str, int] = {}
        self.start_time = time.time()
        self.running = True

    @property
    def focused_service_name(self) -> str:
        """Name of the currently focused service."""
        if not self.service_names:
            return ""
        return self.service_names[self.focused_index % len(self.service_names)]

    def _scroll_focused(self, delta: int) -> None:
        """Adjust scroll offset for the currently focused pane."""
        curr = self.focused_service_name
        if curr:
            old = self.scroll_offsets.get(curr, 0)
            self.scroll_offsets[curr] = max(0, old + delta)

    def _scroll_top(self) -> None:
        """Jump to oldest logs in focused pane."""
        curr = self.focused_service_name
        if curr:
            self.scroll_offsets[curr] = 9999999

    def _scroll_bottom(self) -> None:
        """Jump to newest live logs (follow mode) in focused pane."""
        curr = self.focused_service_name
        if curr:
            self.scroll_offsets[curr] = 0

    def run(self) -> int:
        """Run the interactive TUI event loop."""
        if not self.service_names:
            self.console.print("[yellow]⚠ No services registered to run.[/yellow]")
            return 0

        # Start non-blocking keyboard listener thread on POSIX systems
        input_thread = threading.Thread(target=self._input_loop, daemon=True, name="ws-tui-input")
        input_thread.start()

        try:
            with Live(
                self._render_screen(),
                console=self.console,
                screen=True,
                refresh_per_second=5,
                auto_refresh=True,
            ) as live:
                while self.running:
                    live.update(self._render_screen())
                    time.sleep(0.15)
        except (KeyboardInterrupt, SystemExit):
            pass
        finally:
            self.running = False
            self.console.print("\n[bold yellow]Stopping workspace services...[/bold yellow]")
            self.supervisor.stop_all()
            self.console.print("[bold green]✔ All workspace services stopped cleanly.[/bold green]\n")

        return 0

    def _render_screen(self) -> RenderableType:
        """Generate the full TUI screen layout strictly constrained to terminal dimensions."""
        term_height = self.console.height or 24
        # Allocate exact rows: Header (3) + Body (term_height - 6) + Footer (1)
        # Leaving 2 rows of buffer ensures zero terminal scroll overflow
        body_height = max(4, term_height - 6)

        root = Table.grid(expand=True)
        root.add_column(ratio=1)

        # 1. Header Bar (height=3)
        root.add_row(self._render_header())

        # 2. Main Service Panes (height=body_height)
        if self.fullscreen_mode or len(self.service_names) <= 1:
            root.add_row(
                self._render_service_pane(
                    self.focused_service_name,
                    is_focused=True,
                    height=body_height,
                )
            )
        else:
            root.add_row(self._build_multi_pane_grid(body_height))

        # 3. Compact Footer Bar (height=1)
        root.add_row(self._render_footer())

        return root

    def _render_header(self) -> Panel:
        """Render workspace overview header bar."""
        elapsed = time.time() - self.start_time
        mins, secs = divmod(int(elapsed), 60)
        time_str = f"{mins:02d}:{secs:02d}"

        alive_count = sum(1 for s in self.supervisor.services.values() if s.is_alive)
        total_count = len(self.service_names)

        status_text = f"[bold green]● {alive_count}/{total_count} Running[/bold green]" if alive_count > 0 else "[bold red]■ Stopped[/bold red]"

        grid = Table.grid(expand=True)
        grid.add_column(justify="left", ratio=1)
        grid.add_column(justify="center", ratio=1)
        grid.add_column(justify="right", ratio=1)

        title = f"[bold white]WORKSPACE:[/bold white] [bold cyan]{self.workspace_name}[/bold cyan]"
        mode_badge = "[bold green]⌨ INTERACTIVE[/bold green] " if self.interactive_mode else ""
        focus_info = f"{mode_badge}[dim]Focused:[/dim] [bold white][{self.focused_service_name.upper()}][/bold white]"
        runtime = f"{status_text}  [dim white]Uptime: {time_str}[/dim white]"

        grid.add_row(title, focus_info, runtime)

        border_col = "green" if self.interactive_mode else "cyan"
        return Panel(grid, border_style=border_col, box=box.ROUNDED, padding=(0, 1), height=3)

    def _render_footer(self) -> Table:
        """Render single-line compact footer with scroll shortcuts."""
        grid = Table.grid(expand=True)
        grid.add_column(justify="center")

        if self.interactive_mode:
            msg = (
                "[bold green]⌨ INTERACTIVE ACTIVE[/bold green] — "
                f"[bold white]Typing is forwarded to {self.focused_service_name.upper()}[/bold white]  •  "
                "[bold yellow][Esc][/bold yellow] or [bold yellow][Ctrl+X][/bold yellow] to exit"
            )
            grid.add_row(Text.from_markup(msg))
            return grid

        mode_str = "[bold magenta][FULLSCREEN][/bold magenta] " if self.fullscreen_mode else ""

        shortcuts = (
            f"{mode_str}"
            "[bold white][Tab / ←→][/bold white] [dim]Pane[/dim]  •  "
            "[bold white][↑↓ / j k / PgUp PgDn][/bold white] [dim]Scroll[/dim]  •  "
            "[bold white][End / G][/bold white] [dim]Follow[/dim]  •  "
            "[bold green][i / Enter][/bold green] [bold white]Interact[/bold white]  •  "
            "[bold white][f][/bold white] [dim]Fullscreen[/dim]  •  "
            "[bold white][r][/bold white] [dim]Restart[/dim]  •  "
            "[bold white][c][/bold white] [dim]Clear[/dim]  •  "
            "[bold white][q / Ctrl+C][/bold white] [dim]Quit[/dim]"
        )
        grid.add_row(Text.from_markup(shortcuts))
        return grid

    def _build_multi_pane_grid(self, body_height: int) -> Table:
        """Build multi-pane grid layout using Table.grid strictly within body_height."""
        count = len(self.service_names)

        # 2 services: side-by-side equal columns
        if count == 2:
            grid = Table.grid(expand=True)
            grid.add_column(ratio=1)
            grid.add_column(ratio=1)
            p1 = self._render_service_pane(self.service_names[0], is_focused=(self.focused_index % 2 == 0), height=body_height)
            p2 = self._render_service_pane(self.service_names[1], is_focused=(self.focused_index % 2 == 1), height=body_height)
            grid.add_row(p1, p2)
            return grid

        # 3 or 4 services: 2x2 grid
        half_height = max(4, body_height // 2)
        if count <= 4:
            grid = Table.grid(expand=True)
            grid.add_column(ratio=1)
            grid.add_column(ratio=1)

            p1 = self._render_service_pane(self.service_names[0], is_focused=(self.focused_index % count == 0), height=half_height)
            p2 = self._render_service_pane(self.service_names[1], is_focused=(self.focused_index % count == 1), height=half_height)
            grid.add_row(p1, p2)

            p3 = self._render_service_pane(self.service_names[2], is_focused=(self.focused_index % count == 2), height=half_height)
            if count == 4:
                p4 = self._render_service_pane(self.service_names[3], is_focused=(self.focused_index % count == 3), height=half_height)
                grid.add_row(p3, p4)
            else:
                grid.add_row(p3, "")
            return grid

        # 5+ services: 2-column grid with stacked panes
        left_panes = [self.service_names[i] for i in range(0, count, 2)]
        right_panes = [self.service_names[i] for i in range(1, count, 2)]
        row_height = max(4, body_height // max(len(left_panes), 1))

        grid = Table.grid(expand=True)
        grid.add_column(ratio=1)
        grid.add_column(ratio=1)

        for i in range(max(len(left_panes), len(right_panes))):
            s_left = left_panes[i] if i < len(left_panes) else None
            s_right = right_panes[i] if i < len(right_panes) else None

            p_left = self._render_service_pane(s_left, is_focused=(s_left == self.focused_service_name), height=row_height) if s_left else ""
            p_right = self._render_service_pane(s_right, is_focused=(s_right == self.focused_service_name), height=row_height) if s_right else ""
            grid.add_row(p_left, p_right)

        return grid

    def _render_service_pane(self, name: str, is_focused: bool = False, height: int | None = None) -> Panel:
        """Render a single service pane with crisp box.ROUNDED borders, clipped height, and scrollback."""
        service = self.supervisor.services.get(name)
        if not service:
            return Panel(Text("Service not found"), title=name, box=box.ROUNDED, height=height)

        with service.lock:
            raw_lines = list(service.log_buffer)
            status = service.status
            exit_code = service.exit_code
            port = service.detected_port

        # Status badge
        if status == "running":
            status_badge = "[bold green]● RUNNING[/bold green]"
        elif status == "starting":
            status_badge = "[bold yellow]◌ STARTING[/bold yellow]"
        elif status == "stopped":
            status_badge = f"[bold white]■ STOPPED (exit {exit_code or 0})[/bold white]"
        else:
            status_badge = f"[bold red]✘ FAILED (exit {exit_code or 1})[/bold red]"

        port_str = f" [bold magenta]http://localhost:{port}[/bold magenta]" if port else ""

        # Calculate exact number of visible log lines inside inner box
        usable_lines_count = max(1, (height - 2) if height else 15)

        # Sanitize and flatten lines
        flat_lines: list[str] = []
        for raw in raw_lines:
            san = sanitize_terminal_line(raw)
            if san:
                flat_lines.append(san)


        total_lines = len(flat_lines)
        max_offset = max(0, total_lines - usable_lines_count)
        offset = min(self.scroll_offsets.get(name, 0), max_offset)
        self.scroll_offsets[name] = offset

        # Slice visible window based on scroll offset
        if offset > 0:
            end_idx = max(0, total_lines - offset)
            start_idx = max(0, end_idx - usable_lines_count)
            visible = flat_lines[start_idx:end_idx]
            scroll_badge = f" [bold yellow]▲ SCROLLBACK (-{offset} lines / End to follow)[/bold yellow]"
        else:
            visible = flat_lines[-usable_lines_count:] if flat_lines else []
            scroll_badge = ""

        pane_title = f" [bold white]{name.upper()}[/bold white]  {status_badge}{port_str}{scroll_badge} "

        # Color border: bright green if interactive, bright cyan if focused, dim white if inactive
        if is_focused and self.interactive_mode:
            border_style = "bold green"
        elif is_focused:
            border_style = "bold cyan"
        else:
            border_style = "dim white"

        content = "\n".join(visible) if visible else "[dim](waiting for service output...)[/dim]"

        return Panel(
            Text.from_ansi(content, no_wrap=True) if visible else Text.from_markup(content),
            title=pane_title,
            border_style=border_style,
            box=box.ROUNDED,
            padding=(0, 1),
            height=height,
        )

    def _input_loop(self) -> None:
        """Handle non-blocking keyboard input on POSIX systems with interactive forwarding and scrollback."""
        if not sys.stdin.isatty():
            return

        try:
            import termios
            fd = sys.stdin.fileno()
            old_settings = termios.tcgetattr(fd)
            new_settings = termios.tcgetattr(fd)
            # Disable canonical line buffering and local echo for non-blocking key reads
            new_settings[3] &= ~(termios.ICANON | termios.ECHO)
            # CRITICAL: Keep OPOST enabled so that \n returns to column 0 and never drifts!
            new_settings[1] |= termios.OPOST
            new_settings[6][termios.VMIN] = 1
            new_settings[6][termios.VTIME] = 0
            termios.tcsetattr(fd, termios.TCSANOW, new_settings)
        except Exception:
            return

        try:
            while self.running:
                rlist, _, _ = select.select([sys.stdin], [], [], 0.05)
                if rlist:
                    ch = sys.stdin.read(1)
                    if not ch:
                        break

                    # If in Interactive Input Mode, forward keystrokes directly to the service
                    if self.interactive_mode:
                        # Esc (\x1b) or Ctrl+X (\x18) to exit interactive mode
                        if ch in ("\x18", "\x1d"):  # Ctrl+X or Ctrl+]
                            self.interactive_mode = False
                            continue
                        elif ch == "\x1b":
                            # Check if standalone Esc vs arrow key
                            r2, _, _ = select.select([sys.stdin], [], [], 0.05)
                            if not r2:
                                self.interactive_mode = False
                                continue
                            else:
                                seq = sys.stdin.read(2)
                                self.supervisor.send_input(self.focused_service_name, f"\x1b{seq}")
                                continue

                        self.supervisor.send_input(self.focused_service_name, ch)
                        continue

                    # Normal Navigation Mode
                    if ch == "\x1b":
                        r2, _, _ = select.select([sys.stdin], [], [], 0.05)
                        if not r2:
                            # Standalone Esc resets scroll offset to live follow
                            self._scroll_bottom()
                            continue

                        seq = sys.stdin.read(1)
                        if seq == "[":
                            code = sys.stdin.read(1)
                            # Arrows
                            if code == "A":  # Up Arrow -> Scroll up 1 line
                                self._scroll_focused(1)
                            elif code == "B":  # Down Arrow -> Scroll down 1 line
                                self._scroll_focused(-1)
                            elif code == "C":  # Right Arrow -> Next pane
                                self.focused_index = (self.focused_index + 1) % len(self.service_names)
                            elif code == "D":  # Left Arrow -> Prev pane
                                self.focused_index = (self.focused_index - 1) % len(self.service_names)
                            elif code == "Z":  # Shift+Tab -> Prev pane
                                self.focused_index = (self.focused_index - 1) % len(self.service_names)
                            elif code in ("5", "6", "1", "4", "7", "8"):  # Extended escape codes
                                tilde = sys.stdin.read(1)
                                if code == "5":  # PageUp -> Scroll up 15 lines
                                    self._scroll_focused(15)
                                elif code == "6":  # PageDown -> Scroll down 15 lines
                                    self._scroll_focused(-15)
                                elif code in ("1", "7", "H"):  # Home -> Jump to top
                                    self._scroll_top()
                                elif code in ("4", "8", "F"):  # End -> Jump to bottom
                                    self._scroll_bottom()
                            elif code == "H":  # Home
                                self._scroll_top()
                            elif code == "F":  # End
                                self._scroll_bottom()
                        continue

                    # Scrolling single keys
                    if ch in ("k", "K"):  # Scroll up 1 line
                        self._scroll_focused(1)
                    elif ch in ("j", "J"):  # Scroll down 1 line
                        self._scroll_focused(-1)
                    elif ch in ("u", "U", "\x15", "\x02"):  # Ctrl+U / PageUp -> Scroll up 15 lines
                        self._scroll_focused(15)
                    elif ch in ("d", "D", "\x04", "\x06"):  # Ctrl+D / PageDown -> Scroll down 15 lines
                        self._scroll_focused(-15)
                    elif ch == "g":  # Home -> Jump to top
                        self._scroll_top()
                    elif ch == "G":  # End -> Jump to bottom
                        self._scroll_bottom()

                    # Navigation & Controls
                    elif ch in ("\t", " "):  # Tab / Space -> next pane
                        self.focused_index = (self.focused_index + 1) % len(self.service_names)
                    elif ch in ("i", "I", "\r", "\n"):  # i or Enter -> enter Interactive Input Mode
                        self.interactive_mode = True
                    elif ch.isdigit() and int(ch) >= 1 and int(ch) <= len(self.service_names):
                        self.focused_index = int(ch) - 1
                    elif ch in ("f", "F"):
                        self.fullscreen_mode = not self.fullscreen_mode
                    elif ch in ("r", "R"):
                        curr = self.focused_service_name
                        if curr:
                            self.supervisor.restart_service(curr)
                    elif ch in ("c", "C"):
                        curr = self.focused_service_name
                        service = self.supervisor.services.get(curr)
                        if service:
                            with service.lock:
                                service.log_buffer.clear()
                            self.scroll_offsets[curr] = 0
                    elif ch in ("q", "Q", "\x03"):  # q or Ctrl+C
                        self.running = False
                        break
        except Exception as e:
            logger.debug("TUI input loop error: %s", e)
        finally:
            try:
                termios.tcsetattr(fd, termios.TCSADRAIN, old_settings)
            except Exception:
                pass
