/// Unix domain socket IPC daemon and client for background and detachable workspace session management.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaneRequest {
    pub service: String,
    pub scrollback_offset: usize,
    pub horizontal_offset: usize,
    pub height: usize,
    pub width: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaneResponse {
    pub service: String,
    pub rows: Vec<Vec<u8>>,
    pub actual_offset: usize,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum DaemonRequest {
    Ping,
    GetState,
    PollState {
        panes: Vec<PaneRequest>,
    },
    GetFormattedRows {
        service: String,
        scrollback_offset: usize,
        horizontal_offset: usize,
        height: usize,
        width: usize,
    },
    SendInput {
        service: String,
        data: Vec<u8>,
    },
    RestartService {
        service: String,
    },
    ClearBuffer {
        service: String,
    },
    StopAll,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceInfo {
    pub name: String,
    pub status: String,
    pub port: u16,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum DaemonResponse {
    Pong,
    State {
        services: Vec<ServiceInfo>,
    },
    PollResult {
        services: Vec<ServiceInfo>,
        panes: HashMap<String, PaneResponse>,
    },
    FormattedRows {
        rows: Vec<Vec<u8>>,
        actual_offset: usize,
    },
    Success,
    Error {
        message: String,
    },
}

pub struct SessionDaemon {
    pub workspace_name: String,
    pub supervisor: Arc<ProcessSupervisor>,
    pub socket_path: PathBuf,
}

impl SessionDaemon {
    pub fn new(workspace_name: String, supervisor: Arc<ProcessSupervisor>, socket_path: PathBuf) -> Self {
        Self {
            workspace_name,
            supervisor,
            socket_path,
        }
    }

    pub async fn run(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if self.socket_path.exists() {
            let _ = std::fs::remove_file(&self.socket_path);
        }
        if let Some(parent) = self.socket_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let listener = UnixListener::bind(&self.socket_path)?;
        let (shutdown_tx, mut shutdown_rx) = tokio::sync::mpsc::channel::<()>(1);

        loop {
            tokio::select! {
                _ = shutdown_rx.recv() => {
                    break;
                }
                res = listener.accept() => {
                    match res {
                        Ok((stream, _)) => {
                            let supervisor = Arc::clone(&self.supervisor);
                            let s_tx = shutdown_tx.clone();
                            tokio::spawn(async move {
                                let _ = Self::handle_connection(stream, supervisor, s_tx).await;
                            });
                        }
                        Err(_) => {
                            tokio::time::sleep(Duration::from_millis(50)).await;
                        }
                    }
                }
            }
        }

        self.supervisor.stop_all().await;
        let _ = std::fs::remove_file(&self.socket_path);
        Ok(())
    }


    async fn handle_connection(
        mut stream: UnixStream,
        supervisor: Arc<ProcessSupervisor>,
        shutdown_tx: tokio::sync::mpsc::Sender<()>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let (reader, mut writer) = stream.split();
        let mut buf_reader = BufReader::new(reader);
        let mut line = String::new();

        while buf_reader.read_line(&mut line).await? > 0 {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                line.clear();
                continue;
            }

            let req: Result<DaemonRequest, _> = serde_json::from_str(trimmed);
            let mut should_stop_daemon = false;
            let resp = match req {
                Ok(DaemonRequest::Ping) => DaemonResponse::Pong,
                Ok(DaemonRequest::GetState) => {
                    let svcs = Self::get_service_infos(&supervisor);
                    DaemonResponse::State { services: svcs }
                }
                Ok(DaemonRequest::PollState { panes }) => {
                    let svcs = Self::get_service_infos(&supervisor);
                    let mut pane_map = HashMap::new();

                    for p in panes {
                        if let Some(s) = supervisor.services.get(&p.service) {
                            if let Ok(mut buf) = s.buffer.try_write() {
                                let (rows, actual_offset) = buf.get_formatted_rows(
                                    p.scrollback_offset,
                                    p.horizontal_offset,
                                    p.height,
                                    p.width,
                                );
                                pane_map.insert(
                                    p.service.clone(),
                                    PaneResponse {
                                        service: p.service,
                                        rows,
                                        actual_offset,
                                    },
                                );
                            }
                        }
                    }
                    DaemonResponse::PollResult {
                        services: svcs,
                        panes: pane_map,
                    }
                }
                Ok(DaemonRequest::GetFormattedRows {
                    service,
                    scrollback_offset,
                    horizontal_offset,
                    height,
                    width,
                }) => {
                    if let Some(s) = supervisor.services.get(&service) {
                        if let Ok(mut buf) = s.buffer.try_write() {
                            let (rows, actual_offset) =
                                buf.get_formatted_rows(scrollback_offset, horizontal_offset, height, width);
                            DaemonResponse::FormattedRows { rows, actual_offset }
                        } else {
                            DaemonResponse::FormattedRows {
                                rows: vec![],
                                actual_offset: 0,
                            }
                        }
                    } else {
                        DaemonResponse::Error {
                            message: format!("Service '{}' not found", service),
                        }
                    }
                }
                Ok(DaemonRequest::SendInput { service, data }) => {
                    supervisor.send_input(&service, &data).await;
                    DaemonResponse::Success
                }
                Ok(DaemonRequest::RestartService { service }) => {
                    supervisor.start_service(&service).await;
                    DaemonResponse::Success
                }
                Ok(DaemonRequest::ClearBuffer { service }) => {
                    if let Some(s) = supervisor.services.get(&service) {
                        if let Ok(mut buf) = s.buffer.try_write() {
                            buf.clear();
                        }
                    }
                    DaemonResponse::Success
                }
                Ok(DaemonRequest::StopAll) => {
                    supervisor.stop_all().await;
                    should_stop_daemon = true;
                    DaemonResponse::Success
                }
                Err(e) => DaemonResponse::Error {
                    message: e.to_string(),
                },
            };

            let json_resp = serde_json::to_string(&resp)?;
            writer.write_all(json_resp.as_bytes()).await?;
            writer.write_all(b"\n").await?;
            writer.flush().await?;

            if should_stop_daemon {
                let _ = shutdown_tx.send(()).await;
                break;
            }

            line.clear();
        }

        Ok(())
    }


    fn get_service_infos(supervisor: &ProcessSupervisor) -> Vec<ServiceInfo> {
        let mut svcs = Vec::new();
        for (name, s) in &supervisor.services {
            let status_str = match s.status.try_read() {
                Ok(st) => match &*st {
                    ServiceStatus::Running => "Running".to_string(),
                    ServiceStatus::Starting => "Starting".to_string(),
                    ServiceStatus::Stopped(c) => format!("Stopped({})", c),
                    ServiceStatus::Failed(e) => format!("Failed({})", e),
                },
                Err(_) => "Running".to_string(),
            };
            let port = s.detected_port.load(Ordering::Relaxed) as u16;
            svcs.push(ServiceInfo {
                name: name.clone(),
                status: status_str,
                port,
            });
        }
        svcs
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UIMode {
    Navigation,
    Interactive,
    Visual,
}

pub struct AttachedSessionClient {
    pub workspace_name: String,
    pub socket_path: PathBuf,
    pub service_names: Vec<String>,
    pub focused_index: usize,
    pub fullscreen_mode: bool,
    pub mode: UIMode,
    pub visual_anchor: (usize, usize),
    pub cursors: HashMap<String, (usize, usize)>,
    pub scroll_offsets: HashMap<String, usize>,
    pub horizontal_offsets: HashMap<String, usize>,
    pub pane_rects: HashMap<String, Rect>,
    pub cached_panes: HashMap<String, PaneResponse>,
    pub selection: Option<TextSelection>,
    pub copy_toast: Option<(String, Instant)>,
    pub initial_focus: Option<String>,
    pub start_time: Instant,
}

#[derive(Clone, Debug)]
pub struct TextSelection {
    pub service: String,
    pub start: (usize, usize),
    pub end: (usize, usize),
    pub is_selecting: bool,
}

impl AttachedSessionClient {
    pub fn new(
        workspace_name: String,
        socket_path: PathBuf,
        initial_focus: Option<String>,
        initial_fullscreen: bool,
    ) -> Self {
        Self {
            workspace_name,
            socket_path,
            service_names: Vec::new(),
            focused_index: 0,
            fullscreen_mode: initial_fullscreen,
            mode: UIMode::Navigation,
            visual_anchor: (0, 0),
            cursors: HashMap::new(),
            scroll_offsets: HashMap::new(),
            horizontal_offsets: HashMap::new(),
            pane_rects: HashMap::new(),
            cached_panes: HashMap::new(),
            selection: None,
            copy_toast: None,
            initial_focus,
            start_time: Instant::now(),
        }
    }


    pub fn get_cursor(&self, service: &str) -> (usize, usize) {
        self.cursors.get(service).copied().unwrap_or((0, 0))
    }

    pub fn set_cursor(&mut self, service: &str, pos: (usize, usize)) {
        self.cursors.insert(service.to_string(), pos);
    }



    pub async fn run(&mut self) -> Result<i32, Box<dyn std::error::Error>> {
        let stream = match UnixStream::connect(&self.socket_path).await {
            Ok(s) => s,
            Err(e) => {
                let _ = std::fs::remove_file(&self.socket_path);
                return Err(Box::new(e));
            }
        };
        let (reader, mut writer) = stream.into_split();

        let mut buf_reader = BufReader::new(reader);

        // Query initial services
        let state_req = serde_json::to_string(&DaemonRequest::GetState)? + "\n";
        writer.write_all(state_req.as_bytes()).await?;
        writer.flush().await?;

        let mut line = String::new();
        if buf_reader.read_line(&mut line).await? > 0 {
            if let Ok(DaemonResponse::State { services }) = serde_json::from_str::<DaemonResponse>(line.trim()) {
                self.service_names = services.into_iter().map(|s| s.name).collect();
                if let Some(ref focus) = self.initial_focus {
                    if let Some(idx) = self.service_names.iter().position(|s| s == focus) {
                        self.focused_index = idx;
                    }
                }
            }
        }

        enable_raw_mode()?;
        let mut stdout = std::io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;

        let res = self.event_loop(&mut terminal, &mut buf_reader, &mut writer).await;

        disable_raw_mode()?;
        execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture)?;
        terminal.show_cursor()?;

        res
    }

    async fn event_loop(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
        reader: &mut BufReader<tokio::net::unix::OwnedReadHalf>,
        writer: &mut tokio::net::unix::OwnedWriteHalf,
    ) -> Result<i32, Box<dyn std::error::Error>> {
        let tick_rate = Duration::from_millis(50);

        loop {
            // Build pane queries based on active screen pane rectangles
            let mut pane_queries = Vec::new();
            for name in &self.service_names {
                let rect = self.pane_rects.get(name).cloned().unwrap_or(Rect::new(0, 0, 80, 24));
                let usable_height = (rect.height.saturating_sub(2)) as usize;
                let usable_width = (rect.width.saturating_sub(2)) as usize;
                let scrollback_offset = self.scroll_offsets.get(name).copied().unwrap_or(0);
                let horizontal_offset = self.horizontal_offsets.get(name).copied().unwrap_or(0);

                pane_queries.push(PaneRequest {
                    service: name.clone(),
                    scrollback_offset,
                    horizontal_offset,
                    height: usable_height.max(1),
                    width: usable_width.max(1),
                });
            }

            // Fetch atomic poll state (status + rendered ANSI rows for all panes)
            let poll_req = serde_json::to_string(&DaemonRequest::PollState { panes: pane_queries })? + "\n";
            writer.write_all(poll_req.as_bytes()).await?;
            writer.flush().await?;

            let mut state_line = String::new();
            let mut service_infos: Vec<ServiceInfo> = Vec::new();
            if reader.read_line(&mut state_line).await? > 0 {
                if let Ok(DaemonResponse::PollResult { services, panes }) =
                    serde_json::from_str::<DaemonResponse>(state_line.trim())
                {
                    service_infos = services;
                    self.service_names = service_infos.iter().map(|s| s.name.clone()).collect();
                    self.cached_panes = panes;
                }
            }

            // Draw UI
            terminal.draw(|f| {
                self.render_client_ui(f, &service_infos);
            })?;

            if event::poll(tick_rate)? {
                match event::read()? {
                    Event::Mouse(mouse) => match mouse.kind {
                        MouseEventKind::ScrollUp => self.scroll_focused(3),
                        MouseEventKind::ScrollDown => self.scroll_focused(-3),
                        MouseEventKind::ScrollLeft => self.scroll_horizontal(-5),
                        MouseEventKind::ScrollRight => self.scroll_horizontal(5),
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

                        MouseEventKind::Down(MouseButton::Right) => {
                            if let Some(clip_text) = get_from_clipboard() {
                                let focused = self.focused_service_name().to_string();
                                let input_req = serde_json::to_string(&DaemonRequest::SendInput {
                                    service: focused,
                                    data: clip_text.into_bytes(),
                                })? + "\n";
                                writer.write_all(input_req.as_bytes()).await?;
                                writer.flush().await?;
                            }
                        }
                        _ => {}
                    },

                    Event::Key(key) => {
                        if key.kind != KeyEventKind::Press {
                            continue;
                        }

                        // 1. Interactive Mode
                        if self.mode == UIMode::Interactive {
                            if key.code == KeyCode::Esc
                                || (key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('x'))
                            {
                                self.mode = UIMode::Navigation;
                                continue;
                            }

                            // Clipboard paste in interactive mode (Ctrl+V)
                            if (key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('v'))
                                || (key.modifiers.contains(KeyModifiers::CONTROL | KeyModifiers::SHIFT) && key.code == KeyCode::Char('V'))
                            {
                                if let Some(clip_text) = get_from_clipboard() {
                                    let focused = self.focused_service_name().to_string();
                                    let input_req = serde_json::to_string(&DaemonRequest::SendInput {
                                        service: focused,
                                        data: clip_text.into_bytes(),
                                    })? + "\n";
                                    writer.write_all(input_req.as_bytes()).await?;
                                    writer.flush().await?;
                                }
                                continue;
                            }

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
                                let input_req = serde_json::to_string(&DaemonRequest::SendInput {
                                    service: focused,
                                    data: bytes,
                                })? + "\n";
                                writer.write_all(input_req.as_bytes()).await?;
                                writer.flush().await?;
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
                                // Exit visual mode / cancel selection
                                KeyCode::Esc | KeyCode::Char('v') => {
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

                        // 3. Navigation mode
                        if key.modifiers.contains(KeyModifiers::CONTROL)
                            && (key.code == KeyCode::Char('c')
                                || key.code == KeyCode::Char('q')
                                || key.code == KeyCode::Char('x'))
                        {
                            let req = serde_json::to_string(&DaemonRequest::StopAll)? + "\n";
                            writer.write_all(req.as_bytes()).await?;
                            writer.flush().await?;
                            let mut resp_line = String::new();
                            let _ = reader.read_line(&mut resp_line).await;
                            return Ok(0);
                        }


                        let focused = self.focused_service_name().to_string();
                        let rect = self.pane_rects.get(&focused).cloned().unwrap_or(Rect::new(0, 0, 80, 24));
                        let max_col = (rect.width.saturating_sub(2)) as usize;
                        let max_row = (rect.height.saturating_sub(2)) as usize;
                        let (mut col, mut row) = self.get_cursor(&focused);

                        match key.code {
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
                            // Enter Vim Visual Line Selection Mode at current cursor line
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

                            KeyCode::Tab => {
                                if !self.service_names.is_empty() {
                                    self.focused_index = (self.focused_index + 1) % self.service_names.len();
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
                            KeyCode::Char('f') | KeyCode::Char('F') => {
                                self.fullscreen_mode = !self.fullscreen_mode;
                            }
                            KeyCode::Char('i') | KeyCode::Char('I') | KeyCode::Enter => {
                                self.mode = UIMode::Interactive;
                            }
                            KeyCode::Char('r') | KeyCode::Char('R') => {
                                let name = self.focused_service_name().to_string();
                                let req = serde_json::to_string(&DaemonRequest::RestartService { service: name })? + "\n";
                                writer.write_all(req.as_bytes()).await?;
                                writer.flush().await?;
                            }
                            KeyCode::Char('c') | KeyCode::Char('C') => {
                                let name = self.focused_service_name().to_string();
                                let req = serde_json::to_string(&DaemonRequest::ClearBuffer { service: name.clone() })? + "\n";
                                writer.write_all(req.as_bytes()).await?;
                                writer.flush().await?;
                                self.scroll_offsets.insert(name.clone(), 0);
                                self.horizontal_offsets.insert(name, 0);
                            }
                            // Copy in navigation mode (y / Y)
                            KeyCode::Char('y') | KeyCode::Char('Y') => {
                                let focused = self.focused_service_name().to_string();
                                if let Some(ref sel) = self.selection {
                                    if sel.service == focused && sel.start != sel.end {
                                        let text = self.extract_selected_text(&sel.service, sel.start, sel.end);
                                        if !text.is_empty() {
                                            copy_to_clipboard(&text);
                                            self.copy_toast = Some((
                                                "✔ Copied selection to clipboard".to_string(),
                                                Instant::now(),
                                            ));
                                        }
                                    }
                                } else if let Some(pane_resp) = self.cached_panes.get(&focused) {
                                    let mut lines = Vec::new();
                                    for r in &pane_resp.rows {
                                        lines.push(String::from_utf8_lossy(r).to_string());
                                    }
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
                            // Paste in navigation mode
                            KeyCode::Char('p') => {
                                if let Some(clip_text) = get_from_clipboard() {
                                    let focused = self.focused_service_name().to_string();
                                    let input_req = serde_json::to_string(&DaemonRequest::SendInput {
                                        service: focused,
                                        data: clip_text.into_bytes(),
                                    })? + "\n";
                                    writer.write_all(input_req.as_bytes()).await?;
                                    writer.flush().await?;
                                }
                            }
                            // Detach from background session (leaves processes running)
                            KeyCode::Char('d') | KeyCode::Char('D') => {
                                return Ok(0);
                            }
                            // Stop / terminate session completely (q / Q)
                            KeyCode::Char('q') | KeyCode::Char('Q') => {
                                let req = serde_json::to_string(&DaemonRequest::StopAll)? + "\n";
                                writer.write_all(req.as_bytes()).await?;
                                writer.flush().await?;
                                let mut resp_line = String::new();
                                let _ = reader.read_line(&mut resp_line).await;
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

    fn render_client_ui(&mut self, f: &mut Frame, services: &[ServiceInfo]) {
        let terminal_size = f.size();
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),                                  // Header
                Constraint::Length(terminal_size.height.saturating_sub(5)), // Main Panes
                Constraint::Length(2),                                  // Footer Bar
            ])
            .split(terminal_size);

        // Header
        let elapsed = self.start_time.elapsed().as_secs();
        let mins = elapsed / 60;
        let secs = elapsed % 60;
        let running_count = services.iter().filter(|s| s.status == "Running").count();
        let total = services.len();

        let toast_span = if let Some((msg, created)) = &self.copy_toast {
            if created.elapsed() < Duration::from_secs(3) {
                Span::styled(format!("  {} ", msg), Style::default().fg(Color::Green).add_modifier(Modifier::BOLD))
            } else {
                Span::raw("")
            }
        } else {
            Span::raw("")
        };

        let mode_badge = match self.mode {
            UIMode::Visual => Span::styled(
                " 👁 VISUAL ",
                Style::default().fg(Color::Black).bg(Color::Magenta).add_modifier(Modifier::BOLD),
            ),
            UIMode::Interactive => Span::styled(
                " ⌨ INTERACTIVE ",
                Style::default().fg(Color::Black).bg(Color::Green).add_modifier(Modifier::BOLD),
            ),
            UIMode::Navigation => Span::raw(""),
        };

        let title_line = Line::from(vec![
            Span::styled(
                format!(" WORKSPACE: {} [ATTACHED] ", self.workspace_name),
                Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
            ),
            mode_badge,
            Span::styled(
                format!(" Focused: [{}]", self.focused_service_name().to_uppercase()),
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" ● {}/{} Running", running_count, total),
                Style::default().fg(Color::Green),
            ),
            toast_span,
            Span::styled(
                format!("  Session: {:02}:{:02} ", mins, secs),
                Style::default().fg(Color::DarkGray),
            ),
        ]);

        let border_color = match self.mode {
            UIMode::Visual => Color::Magenta,
            UIMode::Interactive => Color::Green,
            UIMode::Navigation => Color::Cyan,
        };

        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(border_color))
            .title(title_line);

        f.render_widget(block, chunks[0]);

        // Main Body Panes
        let count = services.len();
        if count == 0 {
            return;
        }

        if self.fullscreen_mode || count == 1 {
            let name = self.focused_service_name().to_string();
            self.render_client_pane(f, chunks[1], &name, true, services);
        } else if count == 2 {
            let cols = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                .split(chunks[1]);
            self.render_client_pane(f, cols[0], &services[0].name, self.focused_index % 2 == 0, services);
            self.render_client_pane(f, cols[1], &services[1].name, self.focused_index % 2 == 1, services);
        } else {
            let half = (count + 1) / 2;
            let cols = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                .split(chunks[1]);

            let left_rows = Layout::default()
                .direction(Direction::Vertical)
                .constraints(vec![Constraint::Ratio(1, half as u32); half])
                .split(cols[0]);
            let right_rows = Layout::default()
                .direction(Direction::Vertical)
                .constraints(vec![Constraint::Ratio(1, (count - half) as u32); count - half])
                .split(cols[1]);

            let focused_name = self.focused_service_name().to_string();
            for i in 0..half {
                self.render_client_pane(f, left_rows[i], &services[i].name, services[i].name == focused_name, services);
            }
            for i in half..count {
                self.render_client_pane(f, right_rows[i - half], &services[i].name, services[i].name == focused_name, services);
            }
        }

        // Footer Bar (Always clearly visible with distinct background)
        let footer_line = match self.mode {
            UIMode::Visual => Line::from(vec![
                Span::styled(" 👁 VISUAL ", Style::default().fg(Color::Black).bg(Color::Magenta).add_modifier(Modifier::BOLD)),
                Span::styled(" [h/j/k/l or ↑↓←→]", Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                Span::styled(" Move Cursor  •  ", Style::default().fg(Color::DarkGray)),
                Span::styled("[y/Enter]", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
                Span::styled(" Yank/Copy  •  ", Style::default().fg(Color::DarkGray)),
                Span::styled("[Esc/v]", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
                Span::styled(" Cancel  •  ", Style::default().fg(Color::DarkGray)),
                Span::styled("[0/$]", Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                Span::styled(" Line Start/End", Style::default().fg(Color::DarkGray)),
            ]),
            UIMode::Interactive => Line::from(vec![
                Span::styled(" ⌨ INTERACTIVE ", Style::default().fg(Color::Black).bg(Color::Green).add_modifier(Modifier::BOLD)),
                Span::styled(format!(" Forwarding input to {}  •  ", self.focused_service_name().to_uppercase()), Style::default().fg(Color::White)),
                Span::styled("[Esc]", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
                Span::styled(" Exit Interactive  •  ", Style::default().fg(Color::DarkGray)),
                Span::styled("[Ctrl+V]", Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                Span::styled(" Paste", Style::default().fg(Color::DarkGray)),
            ]),
            UIMode::Navigation => Line::from(vec![
                Span::styled(" [Tab/Click]", Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                Span::styled(" Focus  •  ", Style::default().fg(Color::DarkGray)),
                Span::styled("[v]", Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD)),
                Span::styled(" Visual Select  •  ", Style::default().fg(Color::DarkGray)),
                Span::styled("[d]", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                Span::styled(" Detach (Keep Running)  •  ", Style::default().fg(Color::DarkGray)),
                Span::styled("[q/Ctrl+C]", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
                Span::styled(" Stop Session  •  ", Style::default().fg(Color::DarkGray)),
                Span::styled("[i/Enter]", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
                Span::styled(" Interact  •  ", Style::default().fg(Color::DarkGray)),
                Span::styled("[y/p]", Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                Span::styled(" Copy/Paste  •  ", Style::default().fg(Color::DarkGray)),
                Span::styled("[↑↓/k j]", Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                Span::styled(" Scroll", Style::default().fg(Color::DarkGray)),
            ]),
        };

        let footer_block = Block::default()
            .borders(Borders::TOP)
            .border_style(Style::default().fg(Color::DarkGray))
            .style(Style::default().bg(Color::Rgb(15, 20, 30)));

        let paragraph = Paragraph::new(footer_line).block(footer_block);
        f.render_widget(paragraph, chunks[2]);
    }

    fn render_client_pane(&mut self, f: &mut Frame, area: Rect, name: &str, is_focused: bool, services: &[ServiceInfo]) {
        self.pane_rects.insert(name.to_string(), area);

        let svc_info = services.iter().find(|s| s.name == name);
        let status_desc = if let Some(info) = svc_info {
            if info.status == "Running" {
                (" ● RUNNING ", Color::Green)
            } else if info.status == "Starting" {
                (" ◌ STARTING ", Color::Yellow)
            } else {
                (" ✘ STOPPED ", Color::Red)
            }
        } else {
            (" ● RUNNING ", Color::Green)
        };

        let port_str = if let Some(info) = svc_info {
            if info.port > 0 {
                format!(" http://localhost:{} ", info.port)
            } else {
                String::new()
            }
        } else {
            String::new()
        };

        let actual_offset = self.cached_panes.get(name).map(|p| p.actual_offset).unwrap_or(0);
        let horiz_offset = self.horizontal_offsets.get(name).copied().unwrap_or(0);

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
                Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
            ),
            Span::styled(status_desc.0, Style::default().fg(status_desc.1).add_modifier(Modifier::BOLD)),
            Span::styled(port_str, Style::default().fg(Color::Magenta)),
            Span::styled(horiz_badge, Style::default().fg(Color::Cyan)),
            Span::styled(scroll_badge, Style::default().fg(Color::Yellow)),
        ]);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(border_color))
            .title(title_line);

        // Convert formatted ANSI byte rows into Ratatui Text Lines
        let mut visible_lines = Vec::new();
        if let Some(pane_resp) = self.cached_panes.get(name) {
            for row in &pane_resp.rows {
                match row.into_text() {
                    Ok(t) => visible_lines.extend(t.lines),
                    Err(_) => {
                        let lossy = String::from_utf8_lossy(row).to_string();
                        visible_lines.push(Line::from(lossy));
                    }
                }
            }
        }

        if visible_lines.is_empty() {
            visible_lines.push(Line::from(Span::styled(
                "(waiting for service output...)",
                Style::default().fg(Color::DarkGray),
            )));
        }

        // 1. Apply visual selection highlight if selection is active on this pane
        if let Some(ref sel) = self.selection {
            if sel.service == name && sel.start != sel.end {
                visible_lines = Self::apply_selection_highlight(visible_lines, sel.start, sel.end);
            }
        }

        // 2. Render virtual cursor if this pane is focused and not in interactive mode
        if is_focused && self.mode != UIMode::Interactive {
            let cursor = self.get_cursor(name);
            visible_lines = Self::apply_cursor_highlight(visible_lines, cursor.0, cursor.1, self.mode);
        }

        let paragraph = Paragraph::new(visible_lines).block(block);
        f.render_widget(paragraph, area);
    }

    pub fn extract_selected_text(&self, service: &str, start: (usize, usize), end: (usize, usize)) -> String {
        let mut visible_lines = Vec::new();
        if let Some(pane_resp) = self.cached_panes.get(service) {
            for row in &pane_resp.rows {
                match row.into_text() {
                    Ok(t) => visible_lines.extend(t.lines),
                    Err(_) => {
                        let lossy = String::from_utf8_lossy(row).to_string();
                        visible_lines.push(Line::from(lossy));
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

}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::{Color, Style};

    use ratatui::text::{Line, Span};

    #[test]
    fn test_apply_selection_highlight_single_line() {
        let line = Line::from(vec![
            Span::raw("Hello "),
            Span::styled("World", Style::default().fg(Color::Green)),
            Span::raw(" of Ratatui!"),
        ]);

        let highlighted = AttachedSessionClient::apply_selection_highlight(vec![line], (6, 0), (11, 0));
        assert_eq!(highlighted.len(), 1);

        // "World" (cols 6..11) should be styled with selection style
        let spans = &highlighted[0].spans;
        assert_eq!(spans[0].content, "Hello ");
        assert_eq!(spans[1].content, "World");
        assert_eq!(spans[1].style.bg, Some(Color::Rgb(50, 95, 175)));
        assert_eq!(spans[2].content, " of Ratatui!");
    }

    #[test]
    fn test_apply_selection_highlight_multi_line() {
        let lines = vec![
            Line::from("First Line 12345"),
            Line::from("Second Line 67890"),
            Line::from("Third Line ABCDE"),
        ];

        let highlighted = AttachedSessionClient::apply_selection_highlight(lines, (6, 0), (11, 1));
        assert_eq!(highlighted.len(), 3);

        // Line 0: cols 6.. should be selected
        assert_eq!(highlighted[0].spans[0].content, "First ");
        assert_eq!(highlighted[0].spans[1].content, "Line 12345");
        assert_eq!(highlighted[0].spans[1].style.bg, Some(Color::Rgb(50, 95, 175)));

        // Line 1: cols 0..11 should be selected
        assert_eq!(highlighted[1].spans[0].content, "Second Line");
        assert_eq!(highlighted[1].spans[0].style.bg, Some(Color::Rgb(50, 95, 175)));
        assert_eq!(highlighted[1].spans[1].content, " 67890");

        // Line 2: unselected
        assert_eq!(highlighted[2].spans[0].content, "Third Line ABCDE");
        assert_eq!(highlighted[2].spans[0].style.bg, None);
    }

    #[test]
    fn test_apply_cursor_highlight_in_middle_of_line() {
        let line = Line::from("Hello World");
        let highlighted = AttachedSessionClient::apply_cursor_highlight(vec![line], 6, 0, UIMode::Navigation);
        assert_eq!(highlighted.len(), 1);
        let spans = &highlighted[0].spans;
        assert_eq!(spans[0].content, "Hello ");
        assert_eq!(spans[1].content, "W");
        assert_eq!(spans[1].style.bg, Some(Color::Rgb(255, 215, 0))); // Gold cursor in Navigation mode
        assert_eq!(spans[2].content, "orld");
    }

    #[test]
    fn test_apply_cursor_highlight_at_empty_line() {
        let line = Line::from("");
        let highlighted = AttachedSessionClient::apply_cursor_highlight(vec![line], 0, 0, UIMode::Visual);
        assert_eq!(highlighted.len(), 1);
        let spans = &highlighted[0].spans;
        assert_eq!(spans[0].content, " ");
        assert_eq!(spans[0].style.bg, Some(Color::Rgb(255, 100, 220))); // Magenta cursor in Visual mode
    }
}
