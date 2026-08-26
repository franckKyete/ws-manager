/// High-performance Ratatui terminal user interface with multi-pane grid, in-pane scrollback,
/// lossless window resizing, horizontal panning, smooth trackpad scrolling, text selection, and clipboard copy/paste.

use std::collections::HashMap;
use std::io::{stdout, Stdout};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers,
        MouseButton, MouseEventKind,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph},
    Frame, Terminal,
};

use ansi_to_tui::IntoText;

use crate::clipboard::{copy_to_clipboard, get_from_clipboard};
use crate::supervisor::{ProcessSupervisor, ServiceStatus};

#[derive(Clone, Debug)]
pub struct TextSelection {
    pub service: String,
    pub start: (usize, usize),
    pub end: (usize, usize),
    pub is_selecting: bool,
}


#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UIMode {
    Navigation,
    Interactive,
    Visual,
}

pub struct WorkspaceTUI<'a> {
    pub workspace_name: String,
    pub supervisor: &'a ProcessSupervisor,
    pub service_names: Vec<String>,
    pub focused_index: usize,
    pub fullscreen_mode: bool,
    pub mode: UIMode,
    pub visual_anchor: (usize, usize),
    pub cursors: HashMap<String, (usize, usize)>,
    pub scroll_offsets: HashMap<String, usize>,
    pub horizontal_offsets: HashMap<String, usize>,
    pub pane_rects: HashMap<String, Rect>,
    pub selection: Option<TextSelection>,
    pub copy_toast: Option<(String, Instant)>,
    pub start_time: Instant,
}

impl<'a> WorkspaceTUI<'a> {
    pub fn new(
        workspace_name: String,
        supervisor: &'a ProcessSupervisor,
        initial_focus: Option<String>,
        initial_fullscreen: bool,
    ) -> Self {
        let service_names: Vec<String> = supervisor.services.keys().cloned().collect();
        let mut focused_index = 0;
        if let Some(focus) = initial_focus {
            if let Some(idx) = service_names.iter().position(|s| s == &focus) {
                focused_index = idx;
            }
        }

        Self {
            workspace_name,
            supervisor,
            service_names,
            focused_index,
            fullscreen_mode: initial_fullscreen,
            mode: UIMode::Navigation,
            visual_anchor: (0, 0),
            cursors: HashMap::new(),
            scroll_offsets: HashMap::new(),
            horizontal_offsets: HashMap::new(),
            pane_rects: HashMap::new(),
            selection: None,
            copy_toast: None,
            start_time: Instant::now(),
        }
    }


    pub fn get_cursor(&self, service: &str) -> (usize, usize) {
        self.cursors.get(service).copied().unwrap_or((0, 0))
    }

    pub fn set_cursor(&mut self, service: &str, pos: (usize, usize)) {
        self.cursors.insert(service.to_string(), pos);
    }



    pub fn focused_service_name(&self) -> &str {
        if self.service_names.is_empty() {
            return "";
        }
        &self.service_names[self.focused_index % self.service_names.len()]
    }

    pub fn scroll_focused(&mut self, delta: i32) {
        let name = self.focused_service_name().to_string();
        if !name.is_empty() {
            let old = self.scroll_offsets.get(&name).copied().unwrap_or(0);
            if delta < 0 {
                let sub = (-delta) as usize;
                self.scroll_offsets.insert(name, old.saturating_sub(sub));
            } else {
                self.scroll_offsets.insert(name, old.saturating_add(delta as usize));
            }
        }
    }

    pub fn scroll_horizontal(&mut self, delta: i32) {
        let name = self.focused_service_name().to_string();
        if !name.is_empty() {
            let old = self.horizontal_offsets.get(&name).copied().unwrap_or(0);
            if delta < 0 {
                let sub = (-delta) as usize;
                self.horizontal_offsets.insert(name, old.saturating_sub(sub));
            } else {
                self.horizontal_offsets.insert(name, old.saturating_add(delta as usize).min(200));
            }
        }
    }

    pub fn scroll_horizontal_reset(&mut self) {
        let name = self.focused_service_name().to_string();
        if !name.is_empty() {
            self.horizontal_offsets.insert(name, 0);
        }
    }

    pub fn scroll_top(&mut self) {
        let name = self.focused_service_name().to_string();
        if !name.is_empty() {
            self.scroll_offsets.insert(name, 999999);
        }
    }

    pub fn scroll_bottom(&mut self) {
        let name = self.focused_service_name().to_string();
        if !name.is_empty() {
            self.scroll_offsets.insert(name, 0);
        }
    }

    pub fn copy_selection_or_focused(&mut self) {
        let focused = self.focused_service_name().to_string();
        if focused.is_empty() {
            return;
        }

        if let Some(service) = self.supervisor.services.get(&focused) {
            if let Ok(buf) = service.buffer.try_read() {
                let lines = buf.get_lines();
                let text = lines.join("\n");
                if !text.is_empty() {
                    copy_to_clipboard(&text);
                    self.copy_toast = Some((
                        format!("✔ Copied {} lines to clipboard", lines.len()),
                        Instant::now(),
                    ));
                }
            }
        }
    }

    pub async fn run(&mut self) -> Result<i32, Box<dyn std::error::Error>> {
        enable_raw_mode()?;
        let mut stdout = stdout();
        // Enable mouse capture for smooth trackpad scrolling, drag selection, and click-to-focus
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;

        let res = self.event_loop(&mut terminal).await;

        disable_raw_mode()?;
        execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture)?;
        terminal.show_cursor()?;

        res
    }

    async fn event_loop(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    ) -> Result<i32, Box<dyn std::error::Error>> {
        let tick_rate = Duration::from_millis(50);

        loop {
            terminal.draw(|f| self.render_ui(f))?;

            if event::poll(tick_rate)? {
                match event::read()? {
                    // Smooth Mouse & Trackpad Handling with Text Selection & Copy/Paste
                    Event::Mouse(mouse) => {
                        match mouse.kind {
                            // Smooth vertical momentum scrolling (3 lines per tick)
                            MouseEventKind::ScrollUp => self.scroll_focused(3),
                            MouseEventKind::ScrollDown => self.scroll_focused(-3),
                            // Horizontal trackpad gesture scrolling (5 columns per tick)
                            MouseEventKind::ScrollLeft => self.scroll_horizontal(-5),
                            MouseEventKind::ScrollRight => self.scroll_horizontal(5),
                            // Click to focus and start text drag selection
                            MouseEventKind::Down(MouseButton::Left) => {
                                let mut hit = None;
                                for (name, rect) in &self.pane_rects {
                                    if mouse.column >= rect.x
                                        && mouse.column < rect.x + rect.width
                                        && mouse.row >= rect.y
                                        && mouse.row < rect.y + rect.height
                                    {
                                        let rel_col = (mouse.column.saturating_sub(rect.x + 1)) as usize;
                                        let rel_row = (mouse.row.saturating_sub(rect.y + 1)) as usize;
                                        let max_col = (rect.width.saturating_sub(2)) as usize;
                                        let max_row = (rect.height.saturating_sub(2)) as usize;
                                        let pos = (rel_col.min(max_col), rel_row.min(max_row));
                                        hit = Some((name.clone(), pos));
                                        break;
                                    }
                                }
                                if let Some((name, pos)) = hit {
                                    if let Some(idx) = self.service_names.iter().position(|s| s == &name) {
                                        self.focused_index = idx;
                                    }
                                    self.set_cursor(&name, pos);
                                    self.visual_anchor = pos;
                                    self.selection = Some(TextSelection {
                                        service: name,
                                        start: pos,
                                        end: pos,
                                        is_selecting: true,
                                    });
                                }
                            }
                            // Mouse drag: expand text selection bounding box
                            MouseEventKind::Drag(MouseButton::Left) => {
                                let mut drag_update = None;
                                if let Some(ref sel) = self.selection {
                                    if let Some(rect) = self.pane_rects.get(&sel.service) {
                                        let rel_col = (mouse.column.saturating_sub(rect.x + 1)) as usize;
                                        let rel_row = (mouse.row.saturating_sub(rect.y + 1)) as usize;
                                        let max_col = (rect.width.saturating_sub(2)) as usize;
                                        let max_row = (rect.height.saturating_sub(2)) as usize;
                                        let pos = (rel_col.min(max_col), rel_row.min(max_row));
                                        drag_update = Some((sel.service.clone(), pos));
                                    }
                                }
                                if let Some((service, pos)) = drag_update {
                                    self.set_cursor(&service, pos);
                                    if let Some(ref mut sel) = self.selection {
                                        sel.end = pos;
                                        sel.is_selecting = true;
                                    }
                                }
                            }

                            // Mouse up: finalize selection and copy to clipboard
                            MouseEventKind::Up(MouseButton::Left) => {
                                let sel_info = self.selection.as_ref().map(|s| (s.service.clone(), s.start, s.end));
                                if let Some((service, start, end)) = sel_info {
                                    if start != end {
                                        let text = self.extract_selected_text(&service, start, end);
                                        if !text.is_empty() {
                                            copy_to_clipboard(&text);
                                            self.copy_toast = Some((
                                                "✔ Copied selection to clipboard".to_string(),
                                                Instant::now(),
                                            ));
                                        }
                                        if let Some(ref mut sel) = self.selection {
                                            sel.is_selecting = false;
                                        }
                                    } else {
                                        self.selection = None;
                                    }
                                }
                            }

                            // Right click: paste clipboard contents into active PTY
                            MouseEventKind::Down(MouseButton::Right) => {
                                if let Some(clip_text) = get_from_clipboard() {
                                    let focused = self.focused_service_name().to_string();
                                    self.supervisor.send_input(&focused, clip_text.as_bytes()).await;
                                }
                            }
                            _ => {}
                        }
                    }

                    // Keyboard Event Handling
                    Event::Key(key) => {
                        if key.kind != KeyEventKind::Press {
                            continue;
                        }

                        // 1. Interactive Input Mode
                        if self.mode == UIMode::Interactive {
                            if key.code == KeyCode::Esc
                                || (key.modifiers.contains(KeyModifiers::CONTROL)
                                    && (key.code == KeyCode::Char('x') || key.code == KeyCode::Char('w') || key.code == KeyCode::Char('W')))
                            {
                                self.mode = UIMode::Navigation;
                                continue;
                            }


                            // Clipboard paste in interactive mode (Ctrl+V or Ctrl+Shift+V)
                            if (key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('v'))
                                || (key.modifiers.contains(KeyModifiers::CONTROL | KeyModifiers::SHIFT)
                                    && key.code == KeyCode::Char('V'))
                            {
                                if let Some(clip_text) = get_from_clipboard() {
                                    let focused = self.focused_service_name().to_string();
                                    self.supervisor.send_input(&focused, clip_text.as_bytes()).await;
                                }
                                continue;
                            }

                            // Forward raw byte to active PTY master
                            let focused = self.focused_service_name().to_string();
                            let bytes: Vec<u8> = match key.code {
                                KeyCode::Char(c) => {
                                    if key.modifiers.contains(KeyModifiers::CONTROL) {
                                        let ctrl_code = (c.to_ascii_lowercase() as u8) - b'a' + 1;
                                        vec![ctrl_code]
                                    } else {
                                        c.to_string().into_bytes()
                                    }
                                }
                                KeyCode::Enter => vec![b'\r'],
                                KeyCode::Backspace => vec![0x08],
                                KeyCode::Tab => vec![b'\t'],
                                KeyCode::Up => b"\x1b[A".to_vec(),
                                KeyCode::Down => b"\x1b[B".to_vec(),
                                KeyCode::Right => b"\x1b[C".to_vec(),
                                KeyCode::Left => b"\x1b[D".to_vec(),
                                _ => vec![],
                            };

                            if !bytes.is_empty() {
                                self.supervisor.send_input(&focused, &bytes).await;
                            }
                            continue;
                        }

                        // 2. Vim Visual Mode
                        if self.mode == UIMode::Visual {
                            let focused = self.focused_service_name().to_string();
                            let rect = self.pane_rects.get(&focused).cloned().unwrap_or(Rect::new(0, 0, 80, 24));
                            let max_col = (rect.width.saturating_sub(2)) as usize;
                            let max_row = (rect.height.saturating_sub(2)) as usize;
                            let (mut col, mut row) = self.get_cursor(&focused);

                            match key.code {
                                // Exit visual mode / cancel selection (Esc, v, or Ctrl+W)
                                KeyCode::Esc | KeyCode::Char('v') => {
                                    self.selection = None;
                                    self.mode = UIMode::Navigation;
                                }
                                KeyCode::Char('w') | KeyCode::Char('W') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                    self.selection = None;
                                    self.mode = UIMode::Navigation;
                                }

                                // Yank / Copy selection
                                KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                                    if let Some(ref sel) = self.selection {
                                        let text = self.extract_selected_text(&sel.service, sel.start, sel.end);
                                        if !text.is_empty() {
                                            copy_to_clipboard(&text);
                                            self.copy_toast = Some((
                                                "✔ Copied selection to clipboard".to_string(),
                                                Instant::now(),
                                            ));
                                        }
                                    }
                                    self.mode = UIMode::Navigation;
                                }
                                // Vim Cursor Navigation
                                KeyCode::Char('h') | KeyCode::Left => {
                                    col = col.saturating_sub(1);
                                }
                                KeyCode::Char('l') | KeyCode::Right => {
                                    col = col.saturating_add(1).min(max_col);
                                }
                                KeyCode::Char('k') | KeyCode::Up => {
                                    if row > 0 {
                                        row = row.saturating_sub(1);
                                    } else {
                                        self.scroll_focused(1);
                                    }
                                }
                                KeyCode::Char('j') | KeyCode::Down => {
                                    if row < max_row {
                                        row = row.saturating_add(1);
                                    } else {
                                        self.scroll_focused(-1);
                                    }
                                }
                                KeyCode::Char('w') => {
                                    col = col.saturating_add(5).min(max_col);
                                }
                                KeyCode::Char('b') => {
                                    col = col.saturating_sub(5);
                                }
                                KeyCode::Char('0') | KeyCode::Char('^') => {
                                    col = 0;
                                }
                                KeyCode::Char('$') => {
                                    col = max_col;
                                }
                                KeyCode::Char('g') | KeyCode::Home => {
                                    row = 0;
                                }
                                KeyCode::Char('G') | KeyCode::End => {
                                    row = max_row;
                                }
                                KeyCode::Char('u') | KeyCode::PageUp => {
                                    row = row.saturating_sub(10);
                                    self.scroll_focused(10);
                                }
                                KeyCode::Char('d') | KeyCode::PageDown => {
                                    row = (row + 10).min(max_row);
                                    self.scroll_focused(-10);
                                }
                                _ => {}
                            }

                            self.set_cursor(&focused, (col, row));
                            if self.mode == UIMode::Visual {
                                self.selection = Some(TextSelection {
                                    service: focused,
                                    start: self.visual_anchor,
                                    end: (col, row),
                                    is_selecting: true,
                                });
                            }
                            continue;
                        }

                        // 3. Navigation Mode
                        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
                            self.supervisor.stop_all().await;
                            return Ok(0);
                        }

                        let focused = self.focused_service_name().to_string();
                        let rect = self.pane_rects.get(&focused).cloned().unwrap_or(Rect::new(0, 0, 80, 24));
                        let max_col = (rect.width.saturating_sub(2)) as usize;
                        let max_row = (rect.height.saturating_sub(2)) as usize;
                        let (mut col, mut row) = self.get_cursor(&focused);

                        match key.code {
                            // Vertical Cursor Movement & Scrolling
                            KeyCode::Char('k') | KeyCode::Up => {
                                if row > 0 {
                                    row = row.saturating_sub(1);
                                } else {
                                    self.scroll_focused(1);
                                }
                            }
                            KeyCode::Char('j') | KeyCode::Down => {
                                if row < max_row {
                                    row = row.saturating_add(1);
                                } else {
                                    self.scroll_focused(-1);
                                }
                            }
                            KeyCode::Char('h') | KeyCode::Left => {
                                col = col.saturating_sub(1);
                            }
                            KeyCode::Char('l') | KeyCode::Right => {
                                col = col.saturating_add(1).min(max_col);
                            }
                            KeyCode::Char('w') | KeyCode::Char('W') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                self.selection = None;
                                self.scroll_bottom();
                            }
                            KeyCode::Char('w') => {
                                col = col.saturating_add(5).min(max_col);
                            }
                            KeyCode::Char('b') => {
                                col = col.saturating_sub(5);
                            }
                            KeyCode::Char('0') | KeyCode::Char('^') => {
                                col = 0;
                            }
                            KeyCode::Char('$') => {
                                col = max_col;
                            }
                            KeyCode::PageUp | KeyCode::Char('u') => {
                                row = row.saturating_sub(10);
                                self.scroll_focused(15);
                            }
                            KeyCode::PageDown => {
                                row = (row + 10).min(max_row);
                                self.scroll_focused(-15);
                            }
                            KeyCode::Home | KeyCode::Char('g') => {
                                row = 0;
                                self.scroll_top();
                            }
                            KeyCode::End | KeyCode::Char('G') => {
                                row = max_row;
                                self.scroll_bottom();
                            }
                            KeyCode::Esc => {
                                self.selection = None;
                                self.scroll_bottom();
                            }



                            // Enter Vim Visual Selection Mode at current cursor position
                            KeyCode::Char('v') => {
                                self.mode = UIMode::Visual;
                                self.visual_anchor = (col, row);
                                self.selection = Some(TextSelection {
                                    service: focused.clone(),
                                    start: (col, row),
                                    end: (col, row),
                                    is_selecting: true,
                                });
                            }
                            // Enter Vim Visual Line Selection Mode at current cursor row
                            KeyCode::Char('V') => {
                                self.mode = UIMode::Visual;
                                self.visual_anchor = (0, row);
                                col = max_col;
                                self.selection = Some(TextSelection {
                                    service: focused.clone(),
                                    start: (0, row),
                                    end: (max_col, row),
                                    is_selecting: true,
                                });
                            }

                            // Clipboard Copy & Paste in Navigation Mode
                            KeyCode::Char('y') | KeyCode::Char('Y') => self.copy_selection_or_focused(),
                            KeyCode::Char('p') => {
                                if let Some(clip_text) = get_from_clipboard() {
                                    let focused = self.focused_service_name().to_string();
                                    self.supervisor.send_input(&focused, clip_text.as_bytes()).await;
                                }
                            }

                            // Pane Switching
                            KeyCode::Tab => {
                                if !self.service_names.is_empty() {
                                    self.focused_index =
                                        (self.focused_index + 1) % self.service_names.len();
                                }
                            }
                            KeyCode::BackTab => {
                                if !self.service_names.is_empty() {
                                    self.focused_index = if self.focused_index == 0 {
                                        self.service_names.len() - 1
                                    } else {
                                        self.focused_index - 1
                                    };
                                }
                            }
                            KeyCode::Char(c) if c.is_ascii_digit() && c != '0' => {
                                let idx = (c as usize) - ('1' as usize);
                                if idx < self.service_names.len() {
                                    self.focused_index = idx;
                                }
                            }

                            // Fullscreen Toggle
                            KeyCode::Char('f') | KeyCode::Char('F') => {
                                self.fullscreen_mode = !self.fullscreen_mode;
                            }

                            // Interactive Input
                            KeyCode::Char('i') | KeyCode::Char('I') | KeyCode::Enter => {
                                self.mode = UIMode::Interactive;
                            }

                            // Service Restart
                            KeyCode::Char('r') | KeyCode::Char('R') => {
                                let name = self.focused_service_name().to_string();
                                self.supervisor.start_service(&name).await;
                            }

                            // Clear Buffer
                            KeyCode::Char('c') | KeyCode::Char('C') => {
                                let name = self.focused_service_name().to_string();
                                if let Some(service) = self.supervisor.services.get(&name) {
                                    if let Ok(mut buf) = service.buffer.try_write() {
                                        buf.clear();
                                    }
                                }
                                self.scroll_offsets.insert(name.clone(), 0);
                                self.horizontal_offsets.insert(name, 0);
                            }

                            // Detach from session (leaves processes running)
                            KeyCode::Char('d') | KeyCode::Char('D') => {
                                return Ok(2); // 2 = Detached
                            }

                            // Quit / Stop All
                            KeyCode::Char('q') | KeyCode::Char('Q') => {
                                self.supervisor.stop_all().await;
                                return Ok(0);
                            }
                            _ => {}
                        }
                        self.set_cursor(&focused, (col, row));
                    }
                    _ => {}
                }
            }
        }
    }


    fn render_ui(&mut self, f: &mut Frame) {
        let terminal_size = f.size();
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),                                  // Header
                Constraint::Length(terminal_size.height.saturating_sub(5)), // Main Body
                Constraint::Length(2),                                  // Footer
            ])
            .split(terminal_size);

        // 1. Render Header
        self.render_header(f, chunks[0]);

        // 2. Render Service Panes
        self.render_body(f, chunks[1]);


        // 3. Render Footer
        self.render_footer(f, chunks[2]);
    }

    fn render_header(&self, f: &mut Frame, area: Rect) {
        let elapsed = self.start_time.elapsed().as_secs();
        let mins = elapsed / 60;
        let secs = elapsed % 60;

        let total = self.service_names.len();
        let mut running_count = 0;
        for s in self.supervisor.services.values() {
            if let Ok(st) = s.status.try_read() {
                if *st == ServiceStatus::Running {
                    running_count += 1;
                }
            }
        }

        let mode_badge = match self.mode {
            UIMode::Visual => Span::styled(
                " 👁 VISUAL ",
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            ),
            UIMode::Interactive => Span::styled(
                " ⌨ INTERACTIVE ",
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
            UIMode::Navigation => Span::raw(""),
        };

        let toast_span = if let Some((msg, created)) = &self.copy_toast {
            if created.elapsed() < Duration::from_secs(3) {
                Span::styled(
                    format!("  {} ", msg),
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                )
            } else {
                Span::raw("")
            }
        } else {
            Span::raw("")
        };

        let status_color = if running_count > 0 {
            Color::Green
        } else {
            Color::Red
        };
        let status_text = format!(" ● {}/{} Running", running_count, total);

        let border_color = match self.mode {
            UIMode::Visual => Color::Magenta,
            UIMode::Interactive => Color::Green,
            UIMode::Navigation => Color::Cyan,
        };

        let focused_name = self.focused_service_name().to_uppercase();

        let title_line = Line::from(vec![
            Span::styled(
                format!(" WORKSPACE: {} ", self.workspace_name),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            mode_badge,
            Span::styled(
                format!(" Focused: [{}]", focused_name),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(status_text, Style::default().fg(status_color)),
            toast_span,
            Span::styled(
                format!("  Uptime: {:02}:{:02} ", mins, secs),
                Style::default().fg(Color::DarkGray),
            ),
        ]);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(border_color))
            .title(title_line);

        f.render_widget(block, area);
    }

    fn render_body(&mut self, f: &mut Frame, area: Rect) {
        let names = self.service_names.clone();
        let count = names.len();
        let focused_idx = self.focused_index;

        if self.fullscreen_mode || count <= 1 {
            let focused = self.focused_service_name().to_string();
            self.render_service_pane(f, area, &focused, true);
            return;
        }

        if count == 2 {
            let cols = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                .split(area);

            self.render_service_pane(f, cols[0], &names[0], focused_idx % 2 == 0);
            self.render_service_pane(f, cols[1], &names[1], focused_idx % 2 == 1);
            return;
        }

        if count <= 4 {
            let rows = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                .split(area);

            let top_cols = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                .split(rows[0]);

            let bot_cols = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                .split(rows[1]);

            self.render_service_pane(f, top_cols[0], &names[0], focused_idx % count == 0);
            self.render_service_pane(f, top_cols[1], &names[1], focused_idx % count == 1);
            self.render_service_pane(f, bot_cols[0], &names[2], focused_idx % count == 2);
            if count == 4 {
                self.render_service_pane(f, bot_cols[1], &names[3], focused_idx % count == 3);
            }
            return;
        }

        // 5+ services: 2-column layout
        let half = (count + 1) / 2;
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(area);

        let left_constraints = vec![Constraint::Ratio(1, half as u32); half];
        let right_constraints = vec![Constraint::Ratio(1, (count - half) as u32); count - half];

        let left_rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints(left_constraints)
            .split(cols[0]);
        let right_rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints(right_constraints)
            .split(cols[1]);

        let focused_name = self.focused_service_name().to_string();
        for i in 0..half {
            self.render_service_pane(
                f,
                left_rows[i],
                &names[i],
                names[i] == focused_name,
            );
        }
        for i in half..count {
            self.render_service_pane(
                f,
                right_rows[i - half],
                &names[i],
                names[i] == focused_name,
            );
        }
    }

    fn render_service_pane(&mut self, f: &mut Frame, area: Rect, name: &str, is_focused: bool) {
        let service = match self.supervisor.services.get(name) {
            Some(s) => s,
            None => return,
        };

        // Record pane coordinates for mouse click-to-focus hit-testing
        self.pane_rects.insert(name.to_string(), area);

        let status_desc = if let Ok(st) = service.status.try_read() {
            match &*st {
                ServiceStatus::Running => (" ● RUNNING ", Color::Green),
                ServiceStatus::Starting => (" ◌ STARTING ", Color::Yellow),
                ServiceStatus::Stopped(code) => {
                    if *code == 0 {
                        (" ■ STOPPED ", Color::DarkGray)
                    } else {
                        (" ✘ FAILED ", Color::Red)
                    }
                }
                ServiceStatus::Failed(_) => (" ✘ FAILED ", Color::Red),
            }
        } else {
            (" ● RUNNING ", Color::Green)
        };

        let port_val = service.detected_port.load(Ordering::Relaxed);
        let port_str = if port_val > 0 {
            format!(" http://localhost:{} ", port_val)
        } else {
            String::new()
        };

        let usable_height = area.height.saturating_sub(2) as usize;
        let usable_width = area.width.saturating_sub(2) as usize;

        let requested_offset = *self.scroll_offsets.get(name).unwrap_or(&0);
        let horiz_offset = *self.horizontal_offsets.get(name).unwrap_or(&0);

        let (visible_lines, actual_offset) = if let Ok(mut buf) = service.buffer.try_write() {
            let (formatted_rows, actual_offset) =
                buf.get_formatted_rows(requested_offset, horiz_offset, usable_height, usable_width);
            let mut out = Vec::with_capacity(formatted_rows.len());
            for row in formatted_rows {
                match row.into_text() {
                    Ok(t) => out.extend(t.lines),
                    Err(_) => {
                        let lossy = String::from_utf8_lossy(&row).to_string();
                        out.push(Line::from(lossy));
                    }
                }
            }
            if out.is_empty() {
                (
                    vec![Line::from(Span::styled(
                        "(waiting for service output...)",
                        Style::default().fg(Color::DarkGray),
                    ))],
                    0,
                )
            } else {
                (out, actual_offset)
            }
        } else {
            (
                vec![Line::from(Span::styled(
                    "(buffer locked)",
                    Style::default().fg(Color::DarkGray),
                ))],
                0,
            )
        };

        self.scroll_offsets.insert(name.to_string(), actual_offset);

        let scroll_badge = if actual_offset > 0 {
            format!(" ▲ SCROLLBACK (-{} / End to follow) ", actual_offset)
        } else {
            String::new()
        };

        let horiz_badge = if horiz_offset > 0 {
            format!(" ◀ ▶ (+{} cols) ", horiz_offset)
        } else {
            String::new()
        };

        let border_color = if is_focused && self.mode == UIMode::Visual {
            Color::Magenta
        } else if is_focused && self.mode == UIMode::Interactive {
            Color::Green
        } else if is_focused {
            Color::Cyan
        } else {
            Color::DarkGray
        };

        let title_line = Line::from(vec![
            Span::styled(
                format!(" {} ", name.to_uppercase()),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                status_desc.0,
                Style::default()
                    .fg(status_desc.1)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(port_str, Style::default().fg(Color::Magenta)),
            Span::styled(horiz_badge, Style::default().fg(Color::Cyan)),
            Span::styled(scroll_badge, Style::default().fg(Color::Yellow)),
        ]);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(border_color))
            .title(title_line);

        let mut lines = visible_lines;
        // 1. Apply visual selection highlight if selection is active on this pane
        if let Some(ref sel) = self.selection {
            if sel.service == name && sel.start != sel.end {
                lines = Self::apply_selection_highlight(lines, sel.start, sel.end);
            }
        }

        // 2. Render virtual cursor if this pane is focused and not in interactive mode
        if is_focused && self.mode != UIMode::Interactive {
            let cursor = self.get_cursor(name);
            lines = Self::apply_cursor_highlight(lines, cursor.0, cursor.1, self.mode);
        }

        let paragraph = Paragraph::new(lines).block(block);
        f.render_widget(paragraph, area);
    }



    fn render_footer(&self, f: &mut Frame, area: Rect) {
        let footer_line = match self.mode {
            UIMode::Visual => Line::from(vec![
                Span::styled(
                    " 👁 VISUAL ",
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Magenta)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    " [h/j/k/l or ↑↓←→]",
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(" Move Cursor  •  ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    "[y/Enter]",
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(" Yank/Copy  •  ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    "[Esc/v]",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(" Cancel  •  ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    "[0/$]",
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(" Line Start/End", Style::default().fg(Color::DarkGray)),
            ]),
            UIMode::Interactive => Line::from(vec![
                Span::styled(
                    " ⌨ INTERACTIVE ACTIVE ",
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!(
                        " — Typing forwarded to {}  •  ",
                        self.focused_service_name().to_uppercase()
                    ),
                    Style::default().fg(Color::White),
                ),
                Span::styled(
                    "[Esc] / [Ctrl+X]",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(" Exit  •  ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    "[Ctrl+V]",
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(" Paste", Style::default().fg(Color::DarkGray)),
            ]),
            UIMode::Navigation => Line::from(vec![
                Span::styled(
                    " [Tab/Click]",
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(" Focus  •  ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    "[v]",
                    Style::default()
                        .fg(Color::Magenta)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(" Visual Select  •  ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    "[d]",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(" Detach (Keep Running)  •  ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    "[q/Ctrl+C]",
                    Style::default()
                        .fg(Color::Red)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(" Stop Session  •  ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    "[i/Enter]",
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(" Interact  •  ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    "[y/p]",
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(" Copy/Paste  •  ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    "[↑↓/k j]",
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(" Scroll", Style::default().fg(Color::DarkGray)),
            ]),
        };

        let footer_block = Block::default()
            .borders(Borders::TOP)
            .border_style(Style::default().fg(Color::DarkGray))
            .style(Style::default().bg(Color::Rgb(15, 20, 30)));

        let paragraph = Paragraph::new(footer_line).block(footer_block);
        f.render_widget(paragraph, area);
    }

    pub fn extract_selected_text(&self, service: &str, start: (usize, usize), end: (usize, usize)) -> String {
        let mut visible_lines = Vec::new();
        if let Some(svc) = self.supervisor.services.get(service) {
            let rect = self.pane_rects.get(service).cloned().unwrap_or(Rect::new(0, 0, 80, 24));
            let usable_height = (rect.height.saturating_sub(2)) as usize;
            let usable_width = (rect.width.saturating_sub(2)) as usize;
            let requested_offset = self.scroll_offsets.get(service).copied().unwrap_or(0);
            let horiz_offset = self.horizontal_offsets.get(service).copied().unwrap_or(0);

            if let Ok(mut buf) = svc.buffer.try_write() {
                let (formatted_rows, _) =
                    buf.get_formatted_rows(requested_offset, horiz_offset, usable_height, usable_width);
                for row in formatted_rows {
                    match row.into_text() {
                        Ok(t) => visible_lines.extend(t.lines),
                        Err(_) => {
                            let lossy = String::from_utf8_lossy(&row).to_string();
                            visible_lines.push(Line::from(lossy));
                        }
                    }
                }
            }
        }

        let (c1, r1) = start;
        let (c2, r2) = end;
        let ((start_r, start_c), (end_r, end_c)) = if r1 < r2 || (r1 == r2 && c1 <= c2) {
            ((r1, c1), (r2, c2))
        } else {
            ((r2, c2), (r1, c1))
        };

        let mut selected_parts = Vec::new();
        let max_row = end_r.min(visible_lines.len().saturating_sub(1));

        for row_idx in start_r..=max_row {
            if row_idx >= visible_lines.len() {
                break;
            }
            let line = &visible_lines[row_idx];
            let plain_text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
            let chars: Vec<char> = plain_text.chars().collect();
            let total_chars = chars.len();

            let (from_col, to_col) = if start_r == end_r {
                (start_c.min(total_chars), end_c.min(total_chars))
            } else if row_idx == start_r {
                (start_c.min(total_chars), total_chars)
            } else if row_idx == end_r {
                (0, end_c.min(total_chars))
            } else {
                (0, total_chars)
            };

            if from_col < to_col {
                let row_snippet: String = chars[from_col..to_col].iter().collect();
                selected_parts.push(row_snippet);
            } else if start_r != end_r {
                selected_parts.push(String::new());
            }
        }

        selected_parts.join("\n")
    }

    pub fn apply_selection_highlight(
        mut lines: Vec<Line<'static>>,
        start: (usize, usize),
        end: (usize, usize),
    ) -> Vec<Line<'static>> {
        let (c1, r1) = start;
        let (c2, r2) = end;
        let ((start_r, start_c), (end_r, end_c)) = if r1 < r2 || (r1 == r2 && c1 <= c2) {
            ((r1, c1), (r2, c2))
        } else {
            ((r2, c2), (r1, c1))
        };

        let sel_style = Style::default()
            .bg(Color::Rgb(50, 95, 175))
            .fg(Color::Rgb(255, 255, 255))
            .add_modifier(Modifier::BOLD);

        for row_idx in 0..lines.len() {
            if row_idx < start_r || row_idx > end_r {
                continue;
            }

            let (from_col, to_col) = if start_r == end_r {
                (start_c, end_c)
            } else if row_idx == start_r {
                (start_c, usize::MAX)
            } else if row_idx == end_r {
                (0, end_c)
            } else {
                (0, usize::MAX)
            };

            if from_col >= to_col {
                continue;
            }

            let old_spans = std::mem::take(&mut lines[row_idx].spans);
            let mut new_spans = Vec::new();
            let mut cur_col = 0;

            for span in old_spans {
                let span_start = cur_col;
                let span_chars: Vec<char> = span.content.chars().collect();
                let span_len = span_chars.len();
                let span_end = span_start + span_len;
                cur_col += span_len;

                if span_len == 0 {
                    continue;
                }

                if span_end <= from_col || span_start >= to_col {
                    new_spans.push(span);
                } else {
                    // 1. Part before selection
                    if from_col > span_start {
                        let before_str: String = span_chars[0..(from_col - span_start)].iter().collect();
                        new_spans.push(Span::styled(before_str, span.style));
                    }

                    // 2. Selected part
                    let sel_start_idx = if from_col > span_start { from_col - span_start } else { 0 };
                    let sel_end_idx = if to_col < span_end { to_col - span_start } else { span_len };
                    if sel_start_idx < sel_end_idx {
                        let sel_str: String = span_chars[sel_start_idx..sel_end_idx].iter().collect();
                        new_spans.push(Span::styled(sel_str, sel_style));
                    }

                    // 3. Part after selection
                    if to_col < span_end {
                        let after_str: String = span_chars[(to_col - span_start)..span_len].iter().collect();
                        new_spans.push(Span::styled(after_str, span.style));
                    }
                }
            }

            // If line is empty or shorter than from_col/to_col in multi-line selection
            if cur_col < to_col && row_idx < end_r {
                new_spans.push(Span::styled(" ", sel_style));
            }

            lines[row_idx].spans = new_spans;
        }

        lines
    }

    pub fn apply_cursor_highlight(
        mut lines: Vec<Line<'static>>,
        cursor_col: usize,
        cursor_row: usize,
        mode: UIMode,
    ) -> Vec<Line<'static>> {
        if lines.is_empty() {
            return lines;
        }

        let target_row = cursor_row.min(lines.len().saturating_sub(1));
        let line = &mut lines[target_row];
        let old_spans = std::mem::take(&mut line.spans);
        let mut new_spans = Vec::new();
        let mut cur_col = 0;
        let mut cursor_rendered = false;

        let cursor_style = match mode {
            UIMode::Visual => Style::default()
                .bg(Color::Rgb(255, 100, 220))
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD),
            _ => Style::default()
                .bg(Color::Rgb(255, 215, 0))
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD),
        };

        for span in old_spans {
            if span.content.is_empty() {
                continue;
            }
            let span_start = cur_col;
            let span_chars: Vec<char> = span.content.chars().collect();
            let span_len = span_chars.len();
            let span_end = span_start + span_len;
            cur_col += span_len;


            if !cursor_rendered && cursor_col >= span_start && cursor_col < span_end {
                let offset = cursor_col - span_start;
                // 1. Before cursor
                if offset > 0 {
                    let before_str: String = span_chars[0..offset].iter().collect();
                    new_spans.push(Span::styled(before_str, span.style));
                }
                // 2. Cursor character
                let cur_char: String = span_chars[offset..=offset].iter().collect();
                new_spans.push(Span::styled(cur_char, cursor_style));
                // 3. After cursor
                if offset + 1 < span_len {
                    let after_str: String = span_chars[(offset + 1)..span_len].iter().collect();
                    new_spans.push(Span::styled(after_str, span.style));
                }
                cursor_rendered = true;
            } else {
                new_spans.push(span);
            }
        }

        if !cursor_rendered {
            new_spans.push(Span::styled(" ", cursor_style));
        }

        line.spans = new_spans;
        lines
    }
}



