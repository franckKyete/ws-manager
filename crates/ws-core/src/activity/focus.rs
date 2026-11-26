use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use serde::Deserialize;

use crate::config::ConfigLoader;
use crate::registry::list_registered_projects;

use super::config::{PresenceRule, StringOrList};
use super::db::{now_ms, ActivityDb};
use super::model::{FocusBucket, FocusInterval};
use super::presence::PresenceState;
use super::ActivityConfig;

#[derive(Debug, Clone, Deserialize, Default)]
pub struct HyprWindow {
    #[serde(default)] pub pid: i32,
    #[serde(default)] pub class: String,
    #[serde(default)] pub title: String,
    #[serde(default, rename = "initialClass")] pub initial_class: String,
    #[serde(default)] pub address: String,
}

#[derive(Debug, Clone, Default)]
pub struct FocusResolution {
    pub project_id: String,
    pub project_name: String,
    pub workspace: String,
    pub repo: String,
    pub source: String,
    pub confidence: String,
    pub category: String,
    pub category_source: String,
    pub app: String,
    pub app_kind: String,
    pub domain: String,
    pub rule: String,
    pub evidence: String,
}

#[derive(Debug, Clone, Default)]
struct ProjectEntry {
    id: String,
    name: String,
    path: PathBuf,
    workspaces_dir: PathBuf,
}

#[derive(Debug, Clone, Default)]
struct ProjectIndex {
    entries: Vec<ProjectEntry>,
    by_name: HashMap<String, usize>,
    by_id: HashMap<String, usize>,
    repo_names: HashMap<String, usize>,
}

impl ProjectIndex {
    fn refresh() -> Self {
        let mut entries = Vec::new();
        let mut by_name = HashMap::new();
        let mut by_id = HashMap::new();
        let mut repo_names = HashMap::new();
        for p in list_registered_projects(None, true) {
            let mut cfg_file = p.join("ws-project.yml");
            if !cfg_file.exists() { cfg_file = p.join("ws-project.yaml"); }
            let Ok(config) = ConfigLoader::load_config(Some(&cfg_file), None, true) else { continue; };
            let project_name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            let project_id = config.activity.get("project_id").and_then(|v| v.as_str()).map(|s| s.to_string())
                .unwrap_or_else(|| format!("path:{}", p.to_string_lossy()));
            let idx = entries.len();
            by_name.insert(normalize_name(&project_name), idx);
            by_id.insert(project_id.clone(), idx);
            let ws_dir = config.workspaces_dir.clone();
            if let Ok(wss) = fs::read_dir(&ws_dir) {
                for ws in wss.flatten().take(40) {
                    if !ws.path().is_dir() { continue; }
                    if let Ok(repos) = fs::read_dir(ws.path()) {
                        for r in repos.flatten().take(40) {
                            if r.path().is_dir() { repo_names.entry(normalize_name(&r.file_name().to_string_lossy())).or_insert(idx); }
                        }
                    }
                }
            }
            entries.push(ProjectEntry { id: project_id, name: project_name, path: p, workspaces_dir: ws_dir });
        }
        Self { entries, by_name, by_id, repo_names }
    }

    fn attribute_path(&self, path: &Path) -> Option<(String, String, String, String)> {
        let mut best: Option<(usize, &ProjectEntry, String, String)> = None;
        for e in &self.entries {
            if path.starts_with(&e.workspaces_dir) {
                if let Ok(rel) = path.strip_prefix(&e.workspaces_dir) {
                    let comps: Vec<_> = rel.components().collect();
                    if !comps.is_empty() {
                        let ws_name = comps[0].as_os_str().to_string_lossy().to_string();
                        let repo = if comps.len() >= 2 { comps[1].as_os_str().to_string_lossy().to_string() } else { String::new() };
                        let score = e.workspaces_dir.as_os_str().len() + ws_name.len() + repo.len();
                        if best.as_ref().map(|b| b.0 < score).unwrap_or(true) { best = Some((score, e, ws_name, repo)); }
                        continue;
                    }
                }
            }
            if path.starts_with(&e.path) {
                let score = e.path.as_os_str().len();
                if best.as_ref().map(|b| b.0 < score).unwrap_or(true) { best = Some((score, e, String::new(), String::new())); }
            }
        }
        best.map(|(_, e, ws, repo)| (e.id.clone(), e.name.clone(), ws, repo))
    }

    fn resolve_ref(&self, name: &str) -> Option<&ProjectEntry> {
        if name.is_empty() { return None; }
        if let Some(idx) = self.by_id.get(name) { return self.entries.get(*idx); }
        let n = normalize_name(name);
        if let Some(idx) = self.by_name.get(&n) { return self.entries.get(*idx); }
        self.entries.iter().find(|e| { let en = normalize_name(&e.name); en == n || en.contains(&n) || n.contains(&en) })
    }
}

fn normalize_name(s: &str) -> String { s.chars().filter(|c| c.is_alphanumeric()).flat_map(|c| c.to_lowercase()).collect() }

fn glob_match(pattern: &str, value: &str) -> bool {
    let p = pattern.to_lowercase(); let v = value.to_lowercase();
    glob_match_bytes(p.as_bytes(), v.as_bytes())
}
fn glob_match_bytes(p: &[u8], v: &[u8]) -> bool {
    let (mut pi, mut vi, mut star_pi, mut star_vi) = (0usize, 0usize, None, 0usize);
    while vi < v.len() {
        if pi < p.len() && (p[pi] == b'?' || p[pi] == v[vi]) { pi += 1; vi += 1; }
        else if pi < p.len() && p[pi] == b'*' { star_pi = Some(pi); star_vi = vi; pi += 1; }
        else if let Some(sp) = star_pi { pi = sp + 1; star_vi += 1; vi = star_vi; }
        else { return false; }
    }
    while pi < p.len() && p[pi] == b'*' { pi += 1; }
    pi == p.len()
}

#[derive(Debug, Clone, Default)]
struct WindowContext {
    app: String,
    app_kind: String,
    app_class_raw: String,
    title: String,
    initial_class: String,
    domain: String,
    url: String,
    project_id: String,
    project_name: String,
    workspace: String,
    repo: String,
    source: String,
    confidence: String,
    tmux_session: String,
    tmux_window: String,
    path: String,
    evidence: String,
    category: String,
    category_source: String,
    rule: String,
}

fn normalize_app(class: &str) -> (&'static str, &'static str) {
    let c = class.to_lowercase();
    let is = |s: &str| c == s || c.contains(s);
    if is("kitty") || is("alacritty") || is("wezterm") || is("foot") || is("ghostty") || is("konsole") || is("gnome-terminal") || is("xfce4-terminal") || c.contains("terminal") { return ("terminal", "terminal"); }
    if is("firefox") || is("librewolf") || c.contains("zen-browser") || c == "zen" { return ("firefox", "browser"); }
    if is("chromium") || is("google-chrome") || is("brave-browser") || is("vivaldi") || is("thorium") { return ("chromium", "browser"); }
    if is("code") || c.contains("vscodium") || c.contains("cursor") { return ("vscode", "editor"); }
    if c == "dev.zed.zed" || c == "zed" { return ("zed", "editor"); }
    if c.contains("jetbrains") || c.contains("idea") || c.contains("pycharm") || c.contains("webstorm") || c.contains("clion") || c.contains("goland") { return ("jetbrains", "editor"); }
    if is("neovide") || is("nvim-qt") { return ("nvim", "editor"); }
    if is("mpv") || is("vlc") || c.contains("celluloid") { return ("media", "media"); }
    if is("steam") || is("lutris") || is("heroic") || c.starts_with("steam_app") { return ("game", "game"); }
    if is("slack") { return ("slack", "chat"); }
    if is("discord") || is("vesktop") || is("webcord") { return ("discord", "chat"); }
    if c.contains("telegram") { return ("telegram", "chat"); }
    if is("signal") { return ("signal", "chat"); }
    if is("thunderbird") || is("evolution") || is("geary") { return ("mail", "mail"); }
    if is("org.gnome.nautilus") || is("nautilus") || is("thunar") || is("dolphin") || is("nemo") || is("pcmanfm") { return ("files", "file_manager"); }
    if c.contains("libreoffice") || c.contains("soffice") || is("typora") || is("obsidian") || is("zathura") || is("evince") || is("okular") { return ("office", "office"); }
    if is("spotify") { return ("spotify", "media"); }
    if c.is_empty() { return ("unknown", "unknown"); }
    ("app", "unknown")
}

fn domain_matches(pattern: &str, domain: &str) -> bool {
    let p = pattern.to_lowercase(); let d = domain.to_lowercase();
    if let Some(suf) = p.strip_prefix("*.") { return d == suf || d.ends_with(&format!(".{}", suf)); }
    glob_match(&p, &d)
}
fn url_or_domain_matches(pattern: &str, domain: &str, url: &str) -> bool {
    let p = pattern.to_lowercase();
    if p.contains('/') || p.contains(':') { return glob_match(&p, &url.to_lowercase()); }
    domain_matches(&p, domain)
}

#[derive(Debug, Clone, Default)]
struct RuleHit { category: Option<String>, project: Option<String>, workspace: Option<String>, repo: Option<String>, require: Option<String>, name: String }

fn rule_matches(rule: &PresenceRule, ctx: &WindowContext) -> bool {
    let m = &rule.match_;
    let has_any = !m.class.is_empty() || !m.title.is_empty() || !m.initial_class.is_empty() || !m.domain.is_empty() || !m.url.is_empty()
        || !m.app_kind.is_empty() || !m.tmux_session.is_empty() || !m.tmux_window.is_empty() || !m.path.is_empty();
    if !has_any { return false; }
    let ok = |f: &StringOrList, v: &str| f.is_empty() || f.matches(v, glob_match);
    let ok_dom = |f: &StringOrList, d: &str| match f { StringOrList::None => true, StringOrList::One(p) => domain_matches(p,d), StringOrList::Many(v) => v.iter().any(|p| domain_matches(p,d)) };
    let ok_url = |f: &StringOrList, d: &str, u: &str| match f { StringOrList::None => true, StringOrList::One(p) => url_or_domain_matches(p,d,u), StringOrList::Many(v) => v.iter().any(|p| url_or_domain_matches(p,d,u)) };
    ok(&m.class, &ctx.app_class_raw) && ok(&m.title, &ctx.title) && ok(&m.initial_class, &ctx.initial_class)
        && ok_dom(&m.domain, &ctx.domain) && ok_url(&m.url, &ctx.domain, &ctx.url)
        && ok(&m.app_kind, &ctx.app_kind) && ok(&m.tmux_session, &ctx.tmux_session) && ok(&m.tmux_window, &ctx.tmux_window) && ok(&m.path, &ctx.path)
}

fn builtin_rules() -> Vec<PresenceRule> {
    use StringOrList::{One as O, Many as M};
    let mr = |m: super::config::PresenceRuleMatch, cat: &str, req: Option<&str>, name: &str| PresenceRule { name: Some(name.into()), match_: m, category: Some(cat.into()), project: None, workspace: None, repo: None, require: req.map(|s| s.into()) };
    let dom = |name:&str, ds: &[&str], cat:&str| mr(super::config::PresenceRuleMatch { domain: M(ds.iter().map(|s| s.to_string()).collect()), ..Default::default() }, cat, None, name);
    let kind = |name:&str, k:&str, cat:&str, req: Option<&str>| mr(super::config::PresenceRuleMatch { app_kind: O(k.into()), ..Default::default() }, cat, req, name);
    vec![
        dom("work-github", &["github.com","*.github.com"], "work"),
        dom("work-gitlab", &["gitlab.com","*.gitlab.com"], "work"),
        dom("work-bitbucket", &["bitbucket.org","*.bitbucket.org"], "work"),
        dom("work-devdocs", &["stackoverflow.com","*.stackoverflow.com","stackexchange.com","*.stackexchange.com","docs.rs","crates.io","npmjs.com","pkg.go.dev","developer.mozilla.org","learn.microsoft.com","docs.python.org","docs.docker.com","kubernetes.io","*.kubernetes.io","cloud.google.com","console.aws.amazon.com","portal.azure.com","vercel.com","*.vercel.com","supabase.com","*.supabase.com","api.openai.com","platform.openai.com"], "work"),
        dom("work-localhost", &["localhost","127.0.0.1","*.localhost","0.0.0.0"], "work"),
        dom("work-comms", &["app.slack.com","*.slack.com","teams.microsoft.com","*.teams.microsoft.com","zoom.us","*.zoom.us","meet.google.com"], "work"),
        dom("leisure-video", &["youtube.com","*.youtube.com","youtu.be","netflix.com","*.netflix.com","twitch.tv","*.twitch.tv","tiktok.com","*.tiktok.com","primevideo.com","*.primevideo.com","disneyplus.com","*.disneyplus.com"], "leisure"),
        dom("leisure-social", &["instagram.com","*.instagram.com","x.com","*.x.com","twitter.com","*.twitter.com","reddit.com","*.reddit.com","facebook.com","*.facebook.com","pinterest.com","*.pinterest.com","threads.net","*.threads.net","news.ycombinator.com"], "leisure"),
        dom("neutral-mail", &["mail.google.com","outlook.live.com","outlook.office.com","*.proton.me","mail.proton.me"], "neutral"),
        dom("neutral-info", &["en.wikipedia.org","*.wikipedia.org","google.com","*.google.com","bing.com","duckduckgo.com","chatgpt.com","claude.ai"], "neutral"),
        dom("neutral-chat", &["web.whatsapp.com","discord.com","*.discord.com","web.telegram.org","app.element.io"], "neutral"),
        kind("work-terminal-project", "terminal", "work", Some("project")),
        kind("work-editor-project", "editor", "work", Some("project")),
        kind("work-office-project", "office", "work", Some("project")),
        kind("leisure-media", "media", "leisure", None),
        kind("leisure-game", "game", "leisure", None),
        kind("neutral-chat-app", "chat", "neutral", None),
        kind("neutral-mail-app", "mail", "neutral", None),
        kind("neutral-files", "file_manager", "neutral", None),
    ]
}

fn read_cwd(pid: i32) -> Option<PathBuf> { fs::read_link(format!("/proc/{}/cwd", pid)).ok() }
fn read_comm(pid: i32) -> String { fs::read_to_string(format!("/proc/{}/comm", pid)).unwrap_or_default().trim().to_string() }

fn descendant_pids(pid: i32, depth: usize) -> Vec<i32> {
    if depth == 0 { return Vec::new(); }
    let mut out = Vec::new(); let mut queue = vec![pid];
    for _ in 0..depth {
        let mut next = Vec::new();
        for p in &queue {
            for child in child_pids(*p) { if !out.contains(&child) { out.push(child); next.push(child); } }
        }
        if next.is_empty() { break; }
        queue = next;
    }
    out
}
fn child_pids(pid: i32) -> Vec<i32> {
    let mut children = Vec::new();
    if let Ok(entries) = fs::read_dir(format!("/proc/{}/task", pid)) {
        for ent in entries.flatten() {
            if let Ok(content) = fs::read_to_string(ent.path().join("children")) {
                for tok in content.split_whitespace() { if let Ok(c) = tok.parse::<i32>() { children.push(c); } }
            }
        }
    }
    if !children.is_empty() { return children; }
    if let Ok(entries) = fs::read_dir("/proc") {
        for ent in entries.flatten() {
            let Ok(cpid) = ent.file_name().to_string_lossy().parse::<i32>() else { continue; };
            if let Ok(stat) = fs::read_to_string(ent.path().join("stat")) {
                if let Some(paren) = stat.rfind(')') {
                    let rest = &stat[paren+2..]; let parts: Vec<&str> = rest.split_whitespace().collect();
                    if parts.len() > 1 && parts[1].parse::<i32>().ok() == Some(pid) { children.push(cpid); }
                }
            }
        }
    }
    children
}

#[derive(Debug, Clone, Default)]
struct TmuxContext { session: String, window: String, _pane_cmd: String, pane_path: PathBuf }

fn resolve_tmux(window_pid: i32) -> Option<TmuxContext> {
    let desc = descendant_pids(window_pid, 6);
    let has_tmux = desc.iter().any(|p| read_comm(*p) == "tmux");
    // Even without tmux in descendant tree, still try active client if this is a terminal - caller decides
    let out = std::process::Command::new("tmux").args(["list-clients","-F","#{client_pid}|#{client_tty}|#{client_session}|#{client_activity}"]).output().ok()?;
    if !out.status.success() { return None; }
    let stdout = String::from_utf8_lossy(&out.stdout);
    let mut best: Option<(i64,bool,String)> = None; // (activity, exact, session)
    for line in stdout.lines() {
        let parts: Vec<&str> = line.splitn(4,'|').collect();
        if parts.len() < 4 { continue; }
        let cpid: i32 = parts[0].parse().unwrap_or(0);
        let session = parts[2].to_string();
        let activity: i64 = parts[3].parse().unwrap_or(0);
        let in_tree = cpid == window_pid || desc.contains(&cpid);
        if in_tree { best = Some((i64::MAX, true, session)); break; }
        if !has_tmux { continue; } // only use active client fallback if tmux is involved
        if best.as_ref().map(|b| b.0 < activity).unwrap_or(true) { best = Some((activity,false,session)); }
    }
    // If no client pid match but window has tmux descendant, pick most active client
    if best.is_none() && has_tmux {
        for line in stdout.lines() {
            let parts: Vec<&str> = line.splitn(4,'|').collect();
            if parts.len() < 4 { continue; }
            let session = parts[2].to_string();
            let activity: i64 = parts[3].parse().unwrap_or(0);
            if best.as_ref().map(|b| b.0 < activity).unwrap_or(true) { best = Some((activity,false,session)); }
        }
    }
    let (_, _, session) = best?;
    let out = std::process::Command::new("tmux").args(["display-message","-p","-t",&session,"#{window_index}.#{pane_index}|#{pane_current_command}|#{pane_current_path}|#{window_name}"]).output().ok()?;
    if !out.status.success() { return None; }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let parts: Vec<&str> = s.splitn(4,'|').collect();
    if parts.len() < 3 { return None; }
    Some(TmuxContext { session, window: parts.get(3).unwrap_or(&"").to_string(), _pane_cmd: parts.get(1).unwrap_or(&"").to_string(), pane_path: PathBuf::from(parts.get(2).unwrap_or(&"")) })
}

#[derive(Debug, Clone, Default)]
struct BrowserContext { domain: String, matched_by: String }
#[derive(Debug, Clone, Default)]
struct FirefoxCache { profile_path: Option<PathBuf>, profile_checked_at: Option<Instant>, session_mtime: Option<SystemTime>, session_size: u64, session_data: Option<serde_json::Value> }

fn firefox_profile_for_pid(pid: i32) -> Option<PathBuf> {
    let home = dirs::home_dir()?; let ff = home.join(".mozilla/firefox");
    let needle = format!(":+{}", pid);
    if let Ok(entries) = fs::read_dir(&ff) {
        for ent in entries.flatten() {
            let lock = ent.path().join("lock");
            if let Ok(target) = fs::read_link(&lock) {
                if target.to_string_lossy().contains(&needle) { return Some(ent.path()); }
            }
        }
    }
    // fallback to profiles.ini Default=1
    let ini = ff.join("profiles.ini");
    let content = fs::read_to_string(&ini).ok()?;
    let mut cur_path: Option<String> = None; let mut cur_default = false; let mut fallback: Option<PathBuf> = None;
    for line in content.lines().chain(std::iter::once("[_]")) {
        let line = line.trim();
        if line.starts_with('[') {
            if cur_default { if let Some(p) = cur_path.take() { fallback = Some(ff.join(p)); } }
            cur_path = None; cur_default = false; continue;
        }
        if let Some(v) = line.strip_prefix("Path=") { cur_path = Some(v.to_string()); }
        if line == "Default=1" { cur_default = true; }
        if line.starts_with("Default=") && line != "Default=0" { cur_default = true; }
    }
    fallback.filter(|p| p.is_dir())
}

fn strip_firefox_title_suffix(title: &str) -> String {
    let mut t = title.trim().to_string();
    for suf in [" — Mozilla Firefox", " - Mozilla Firefox", " — Firefox Developer Edition", " — LibreWolf", " — Zen Browser", " — Mozilla Firefox Private Browsing"] {
        if t.to_lowercase().ends_with(&suf.to_lowercase()) { t = t[..t.len()-suf.len()].trim().to_string(); break; }
    }
    t
}
fn normalize_title_for_match(s: &str) -> String { s.to_lowercase().split_whitespace().collect::<Vec<_>>().join(" ") }
fn titles_match(window_title: &str, tab_title: &str) -> bool {
    let w = normalize_title_for_match(&strip_firefox_title_suffix(window_title));
    let t = normalize_title_for_match(tab_title);
    if w.is_empty() || t.is_empty() { return false; }
    if w == t || w.starts_with(&t) || t.starts_with(&w) || w.contains(&t) { return true; }
    let w8: String = w.chars().take(40).collect(); let t8: String = t.chars().take(40).collect();
    w8.len() >= 15 && t8.starts_with(&w8)
}
fn extract_domain(url: &str) -> String {
    let u = url.trim();
    let without_scheme = u.split("://").nth(1).unwrap_or(u);
    let host = without_scheme.split('/').next().unwrap_or("").split('@').last().unwrap_or("");
    let host = host.split(':').next().unwrap_or(host).to_lowercase();
    host.strip_prefix("www.").unwrap_or(&host).to_string()
}

fn load_firefox_session<'a>(profile: &Path, cache: &'a mut FirefoxCache) -> Option<&'a serde_json::Value> {
    let sp = profile.join("sessionstore-backups/recovery.jsonlz4");
    let sp = if sp.exists() { sp } else { profile.join("sessionstore-backups/recovery.baklz4") };
    let meta = fs::metadata(&sp).ok()?;
    let mtime = meta.modified().ok();
    let size = meta.len();
    if cache.session_mtime == mtime && cache.session_size == size && cache.session_data.is_some() { return cache.session_data.as_ref(); }
    let raw = fs::read(&sp).ok()?;
    if raw.len() < 12 || &raw[0..8] != b"mozLz40\0" { return None; }
    let out_size = u32::from_le_bytes([raw[8],raw[9],raw[10],raw[11]]) as usize;
    if out_size > 64 * 1024 * 1024 { return None; }
    let decomp = lz4_flex::block::decompress(&raw[12..], out_size).ok()?;
    let v = serde_json::from_slice::<serde_json::Value>(&decomp).ok()?;
    cache.session_mtime = mtime; cache.session_size = size; cache.session_data = Some(v);
    cache.session_data.as_ref()
}

fn find_tab_in_session(v: &serde_json::Value, window_title: &str) -> Option<BrowserContext> {
    let windows = v.get("windows")?.as_array()?;
    let selected_win = v.get("selectedWindow").and_then(|x| x.as_u64()).unwrap_or(1) as usize;
    let mut order: Vec<usize> = (0..windows.len()).collect();
    if selected_win >= 1 && selected_win <= windows.len() { let idx = selected_win - 1; order.retain(|i| *i != idx); order.insert(0, idx); }
    for wi in order {
        let tabs = match windows[wi].get("tabs").and_then(|t| t.as_array()) { Some(t) => t, None => continue };
        for tab in tabs {
            let entries = match tab.get("entries").and_then(|e| e.as_array()) { Some(e) => e, None => continue };
            let idx = tab.get("index").and_then(|x| x.as_u64()).unwrap_or(entries.len() as u64) as usize;
            let ent = if idx >= 1 && idx <= entries.len() { &entries[idx-1] } else if let Some(e) = entries.last() { e } else { continue };
            let title = ent.get("title").and_then(|s| s.as_str()).unwrap_or("");
            let url = ent.get("url").and_then(|s| s.as_str()).unwrap_or("");
            if titles_match(window_title, title) { return Some(BrowserContext { domain: extract_domain(url), matched_by: "sessionstore".to_string() }); }
        }
    }
    None
}

fn read_firefox_history(profile: &Path, window_title: &str) -> Option<BrowserContext> {
    let places = profile.join("places.sqlite");
    if !places.exists() { return None; }
    let uri = format!("file:{}?immutable=1", places.to_string_lossy());
    let conn = rusqlite::Connection::open_with_flags(&uri, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI).ok()?;
    let stripped = strip_firefox_title_suffix(window_title);
    if normalize_title_for_match(&stripped).len() < 6 { return None; }
    let mut stmt = conn.prepare("SELECT url, title FROM moz_places WHERE title = ?1 ORDER BY last_visit_date DESC LIMIT 1").ok()?;
    let mut rows = stmt.query(rusqlite::params![stripped]).ok()?;
    if let Ok(Some(r)) = rows.next() {
        let url: String = r.get(0).unwrap_or_default();
        return Some(BrowserContext { domain: extract_domain(&url), matched_by: "history".to_string() });
    }
    let prefix: String = stripped.chars().take(60).collect();
    let like = format!("{}%", prefix.replace('%',"\\%").replace('_',"\\_"));
    let mut stmt = conn.prepare("SELECT url, title FROM moz_places WHERE title LIKE ?1 ESCAPE '\\' ORDER BY last_visit_date DESC LIMIT 5").ok()?;
    let mut rows = stmt.query(rusqlite::params![like]).ok()?;
    while let Ok(Some(r)) = rows.next() {
        let url: String = r.get(0).unwrap_or_default(); let title: String = r.get(1).unwrap_or_default();
        if titles_match(window_title, &title) { return Some(BrowserContext { domain: extract_domain(&url), matched_by: "history".to_string() }); }
    }
    None
}

fn browser_from_title(title: &str) -> Option<BrowserContext> {
    let t = title.to_lowercase();
    let known: &[(&[&str], &str)] = &[(&["github"],"github.com"),(&["gitlab"],"gitlab.com"),(&["stack overflow","stackoverflow"],"stackoverflow.com"),(&["youtube"],"youtube.com"),(&["instagram"],"instagram.com"),(&["reddit"],"reddit.com"),(&["twitter"," / x"],"x.com"),(&["netflix"],"netflix.com"),(&["twitch"],"twitch.tv"),(&["slack"],"slack.com"),(&["linkedin"],"linkedin.com"),(&["gmail","inbox"],"mail.google.com"),(&["docs.rs"],"docs.rs"),(&["localhost"],"localhost")];
    for (keys, domain) in known { if keys.iter().any(|k| t.contains(k)) { return Some(BrowserContext { domain: domain.to_string(), matched_by: "title".to_string() }); } }
    None
}

fn resolve_browser(win: &HyprWindow, cfg: &ActivityConfig, cache: &mut FirefoxCache) -> Option<BrowserContext> {
    let mode = cfg.presence.browser.inspect.as_str();
    if mode == "off" { return None; }
    let cls = win.class.to_lowercase();
    let is_firefox = cls.contains("firefox") || cls.contains("librewolf") || cls.contains("zen");
    let is_chromium = cls.contains("chromium") || cls.contains("chrome") || cls.contains("brave") || cls.contains("vivaldi");
    if is_firefox {
        if mode == "title" { return browser_from_title(&win.title); }
        if cache.profile_path.is_none() || cache.profile_checked_at.map(|t| t.elapsed() > Duration::from_secs(60)).unwrap_or(true) {
            cache.profile_path = firefox_profile_for_pid(win.pid); cache.profile_checked_at = Some(Instant::now());
            cache.session_data = None; cache.session_mtime = None;
        }
        let profile = cache.profile_path.clone()?;
        if mode == "auto" || mode == "sessionstore" {
            if let Some(v) = load_firefox_session(&profile, cache) { if let Some(ctx) = find_tab_in_session(v, &win.title) { return Some(ctx); } }
        }
        if mode == "auto" || mode == "history" { if let Some(ctx) = read_firefox_history(&profile, &win.title) { return Some(ctx); } }
        return browser_from_title(&win.title);
    }
    if is_chromium { return browser_from_title(&win.title); }
    None
}

fn ensure_hyprland_env() {
    if std::env::var("HYPRLAND_INSTANCE_SIGNATURE").is_ok() { return; }
    let Ok(runtime) = std::env::var("XDG_RUNTIME_DIR") else { return; };
    let hypr_dir = std::path::Path::new(&runtime).join("hypr");
    let Ok(entries) = fs::read_dir(&hypr_dir) else { return; };
    for ent in entries.flatten() {
        let sig = ent.file_name().to_string_lossy().to_string();
        if ent.path().join(".socket2.sock").exists() { std::env::set_var("HYPRLAND_INSTANCE_SIGNATURE", sig); return; }
    }
}

fn query_active_window() -> Option<HyprWindow> {
    let out = std::process::Command::new("hyprctl").args(["activewindow","-j"]).output().ok()?;
    if !out.status.success() { return None; }
    serde_json::from_slice::<HyprWindow>(&out.stdout).ok()
}

pub(crate) fn resolve_window_context(win: &HyprWindow, cfg: &ActivityConfig, idx: &ProjectIndex, firefox: &mut FirefoxCache) -> WindowContext {
    let (app, app_kind) = normalize_app(&win.class);
    let mut ctx = WindowContext {
        app: app.to_string(), app_kind: app_kind.to_string(),
        app_class_raw: win.class.clone(), title: win.title.clone(), initial_class: win.initial_class.clone(),
        ..Default::default()
    };

    // Terminal: tmux active pane is authoritative
    if ctx.app_kind == "terminal" {
        if let Some(tmux) = resolve_tmux(win.pid) {
            ctx.tmux_session = tmux.session.clone();
            ctx.tmux_window = tmux.window.clone();
            ctx.path = tmux.pane_path.to_string_lossy().to_string();
            ctx.evidence = format!("tmux:{}/{}", tmux.session, tmux.window);
            if let Some((pid, pname, ws, repo)) = idx.attribute_path(&tmux.pane_path) {
                ctx.project_id = pid; ctx.project_name = pname; ctx.workspace = ws; ctx.repo = repo;
                ctx.source = "tmux".to_string(); ctx.confidence = "high".to_string();
            } else {
                // try session/window name against project names
                let candidates = [&tmux.session, &tmux.window];
                for cand in candidates {
                    if let Some(e) = idx.resolve_ref(cand) {
                        ctx.project_id = e.id.clone(); ctx.project_name = e.name.clone();
                        ctx.source = "tmux-name".to_string(); ctx.confidence = "medium".to_string();
                        break;
                    }
                }
                if ctx.project_id.is_empty() {
                    ctx.source = "tmux".to_string(); ctx.confidence = "low".to_string();
                }
            }
        }
    }

    // Browser: domain/url
    if ctx.app_kind == "browser" {
        if let Some(b) = resolve_browser(win, cfg, firefox) {
            ctx.domain = b.domain; ctx.url = String::new(); // don't retain full URL
            ctx.source = format!("browser:{}", b.matched_by);
            ctx.confidence = "high".to_string();
            ctx.evidence = format!("browser:{}", b.matched_by);
        } else {
            ctx.source = "browser:title".to_string();
            ctx.confidence = "low".to_string();
        }
    }

    // Generic path attribution (cwd/process/fd) if not resolved
    if ctx.project_id.is_empty() {
        if let Some((pid,pname,ws,repo,src,conf)) = attribute_pid_to_project(win.pid, cfg.presence.terminal_process_tree, idx) {
            ctx.project_id = pid; ctx.project_name = pname; ctx.workspace = ws; ctx.repo = repo;
            ctx.source = src; ctx.confidence = conf;
            if ctx.evidence.is_empty() { ctx.evidence = "proc".to_string(); }
        }
    }
    // Title token project hint for docs/editors when no strong path
    if ctx.project_id.is_empty() && matches!(ctx.app_kind.as_str(), "office"|"editor"|"unknown") {
        if let Some(e) = project_from_title(&win.title, idx) {
            ctx.project_id = e.id.clone(); ctx.project_name = e.name.clone();
            ctx.source = "title".to_string(); ctx.confidence = "low".to_string();
            ctx.evidence = "title".to_string();
        }
    }

    ctx
}

fn project_from_title<'a>(title: &str, idx: &'a ProjectIndex) -> Option<&'a ProjectEntry> {
    let t = normalize_name(title);
    if t.len() < 4 { return None; }
    let mut best: Option<&ProjectEntry> = None;
    let mut best_len = 0usize;
    for e in &idx.entries {
        let n = normalize_name(&e.name);
        if n.len() < 4 { continue; }
        if t.contains(&n) && n.len() > best_len { best = Some(e); best_len = n.len(); }
    }
    if best.is_some() { return best; }
    // try repo names
    for (rn, i) in &idx.repo_names {
        if rn.len() < 4 { continue; }
        if t.contains(rn) && rn.len() > best_len { best = idx.entries.get(*i); best_len = rn.len(); }
    }
    best
}

fn attribute_pid_to_project(pid: i32, walk_tree: bool, idx: &ProjectIndex) -> Option<(String,String,String,String,String,String)> {
    if pid <= 0 { return None; }
    let mut candidates: Vec<(PathBuf, String)> = Vec::new();
    if let Some(cwd) = read_cwd(pid) { candidates.push((cwd, "cwd".to_string())); }
    if walk_tree {
        for child in descendant_pids(pid, 5) {
            if let Some(cwd) = read_cwd(child) { if !candidates.iter().any(|(p,_)| p == &cwd) { candidates.push((cwd, "process".to_string())); } }
        }
    }
    for (path, src) in candidates {
        if let Some((id,name,ws,repo)) = idx.attribute_path(&path) {
            let conf = if src == "cwd" { "high" } else { "medium" };
            return Some((id,name,ws,repo,src,conf.to_string()));
        }
    }
    None
}

pub(crate) fn resolve_focus(win: &HyprWindow, cfg: &ActivityConfig, idx: &ProjectIndex, firefox: &mut FirefoxCache) -> FocusResolution {
    let mut ctx = resolve_window_context(win, cfg, idx, firefox);
    apply_rules(&mut ctx, cfg, idx);
    FocusResolution {
        project_id: ctx.project_id, project_name: ctx.project_name, workspace: ctx.workspace, repo: ctx.repo,
        source: ctx.source, confidence: ctx.confidence, category: ctx.category, category_source: ctx.category_source,
        app: ctx.app, app_kind: ctx.app_kind, domain: ctx.domain, rule: ctx.rule, evidence: ctx.evidence,
    }
}

fn apply_rules(ctx: &mut WindowContext, cfg: &ActivityConfig, idx: &ProjectIndex) {
    // Find matching rules in order: user, then builtin
    let mut user_hit: Option<RuleHit> = None;
    for rule in &cfg.presence.rules {
        if rule_matches(rule, ctx) {
            user_hit = Some(RuleHit { category: rule.category.clone(), project: rule.project.clone(), workspace: rule.workspace.clone(), repo: rule.repo.clone(), require: rule.require.clone(), name: rule.name.clone().unwrap_or_else(|| "user".to_string()) });
            break;
        }
    }
    let mut builtin_hit: Option<RuleHit> = None;
    let builtins = builtin_rules();
    for rule in &builtins {
        if rule_matches(rule, ctx) {
            builtin_hit = Some(RuleHit { category: rule.category.clone(), project: rule.project.clone(), workspace: rule.workspace.clone(), repo: rule.repo.clone(), require: rule.require.clone(), name: rule.name.clone().unwrap_or_else(|| "builtin".to_string()) });
            break;
        }
    }
    let hit = user_hit.as_ref().or(builtin_hit.as_ref());

    // Resolve project/workspace/repo from rule if provided and we don't have one
    if ctx.project_id.is_empty() {
        if let Some(h) = hit {
            if let Some(pref) = &h.project { if let Some(e) = idx.resolve_ref(pref) { ctx.project_id = e.id.clone(); ctx.project_name = e.name.clone(); ctx.source = "rule".to_string(); } }
            if ctx.workspace.is_empty() { ctx.workspace = h.workspace.clone().unwrap_or_default(); }
            if ctx.repo.is_empty() { ctx.repo = h.repo.clone().unwrap_or_default(); }
        }
    }
    let project_mapped = !ctx.project_id.is_empty();

    // Category precedence: project mapping > explicit work rule > explicit leisure rule > default neutral
    if project_mapped {
        let work_capable = matches!(ctx.app_kind.as_str(), "terminal"|"editor"|"office"|"browser"|"unknown"|"mail"|"chat"|"file_manager"|"app") || ctx.source == "tmux" || ctx.source == "tmux-name" || ctx.source == "cwd" || ctx.source == "process" || ctx.source == "title";
        if work_capable {
            ctx.category = "work".to_string();
            ctx.category_source = if hit.as_ref().and_then(|h| h.category.as_deref()) == Some("work") { "work_rule".to_string() } else { "target".to_string() };
            if let Some(h) = hit { if ctx.rule.is_empty() { ctx.rule = h.name.clone(); } }
            if ctx.confidence.is_empty() { ctx.confidence = "medium".to_string(); }
            return;
        }
    }

    if let Some(h) = hit {
        // Handle require: project
        if h.require.as_deref() == Some("project") && !project_mapped {
            ctx.category = "neutral".to_string(); ctx.category_source = "default".to_string();
            ctx.rule = h.name.clone();
            if ctx.source.is_empty() { ctx.source = "unattributed".to_string(); }
            if ctx.confidence.is_empty() { ctx.confidence = "low".to_string(); }
            return;
        }
        let cat = h.category.clone().unwrap_or_else(|| "neutral".to_string());
        ctx.category = cat.clone();
        ctx.category_source = match cat.as_str() { "work" => "work_rule", "leisure" => "leisure_rule", _ => "default" }.to_string();
        ctx.rule = h.name.clone();
        if ctx.source.is_empty() { ctx.source = "rule".to_string(); }
        if ctx.confidence.is_empty() { ctx.confidence = "medium".to_string(); }
        return;
    }

    if project_mapped {
        ctx.category = "work".to_string(); ctx.category_source = "target".to_string();
    } else {
        ctx.category = "neutral".to_string(); ctx.category_source = "default".to_string();
        if ctx.source.is_empty() { ctx.source = "unattributed".to_string(); }
    }
    if ctx.confidence.is_empty() { ctx.confidence = "low".to_string(); }
}

pub struct FocusMonitor {
    current: Option<FocusResolution>,
    current_interval_id: Option<i64>,
    last_poll: Instant,
    last_rollup: Instant,
    last_transition_ts: i64,
    project_index: ProjectIndex,
    last_index_refresh: Instant,
    firefox: FirefoxCache,
}

impl FocusMonitor {
    pub fn new(db: &ActivityDb) -> Self {
        ensure_hyprland_env();
        let now = now_ms();
        let _ = db.close_open_focus_intervals(now);
        Self {
            current: None, current_interval_id: None,
            last_poll: Instant::now() - Duration::from_secs(3600),
            last_rollup: Instant::now() - Duration::from_secs(3600),
            last_transition_ts: now,
            project_index: ProjectIndex::refresh(),
            last_index_refresh: Instant::now(),
            firefox: FirefoxCache::default(),
        }
    }

    pub fn current(&self) -> Option<&FocusResolution> { self.current.as_ref() }

    pub fn tick(&mut self, db: &ActivityDb, cfg: &ActivityConfig, presence_state: PresenceState) {
        let now = now_ms();
        if !cfg.presence.enabled || !cfg.presence.attribution {
            if self.current_interval_id.is_some() { self.close_current(db, now); }
            return;
        }
        if self.last_index_refresh.elapsed() >= Duration::from_secs(60) {
            self.project_index = ProjectIndex::refresh();
            self.last_index_refresh = Instant::now();
        }
        let present = presence_state.is_present();
        if self.last_poll.elapsed() >= Duration::from_secs(2) {
            if present {
                let res = self.resolve_current(db, cfg, now);
                self.update_focus(db, now, res);
            } else if self.current_interval_id.is_some() {
                self.close_current(db, now);
            }
            self.last_poll = Instant::now();
        }
        if self.last_rollup.elapsed() >= Duration::from_secs(15) {
            let _ = rollup_focus(db, now);
            self.last_rollup = Instant::now();
        }
    }

    fn close_current(&mut self, db: &ActivityDb, now: i64) {
        if let Some(id) = self.current_interval_id.take() { let _ = db.close_focus_interval(id, now.max(self.last_transition_ts)); }
        self.current = None; self.last_transition_ts = now;
    }

    fn update_focus(&mut self, db: &ActivityDb, now: i64, res: FocusResolution) {
        let changed = match &self.current {
            None => true,
            Some(c) => c.project_id != res.project_id || c.workspace != res.workspace || c.repo != res.repo
                || c.category != res.category || c.source != res.source || c.app != res.app || c.domain != res.domain,
        };
        if !changed { return; }
        if let Some(id) = self.current_interval_id.take() { let _ = db.close_focus_interval(id, now.max(self.last_transition_ts)); }
        self.last_transition_ts = now;
        self.current = Some(res.clone());
        let fi = FocusInterval {
            id: 0, start_ts: now, end_ts: None,
            project_id: res.project_id, project_name: res.project_name, workspace: res.workspace, repo: res.repo,
            source: res.source, confidence: res.confidence, category: res.category, category_source: res.category_source,
            app: res.app, app_kind: res.app_kind, domain: res.domain, rule: res.rule, evidence: res.evidence,
        };
        if let Ok(id) = db.open_focus_interval(&fi) { self.current_interval_id = Some(id); }
    }

    fn resolve_current(&mut self, db: &ActivityDb, cfg: &ActivityConfig, now: i64) -> FocusResolution {
        let mut res = self.resolve_window(cfg);
        let provisional = FocusInterval {
            id: 0, start_ts: now, end_ts: None,
            project_id: res.project_id.clone(), project_name: res.project_name.clone(), workspace: res.workspace.clone(), repo: res.repo.clone(),
            source: res.source.clone(), confidence: res.confidence.clone(), category: res.category.clone(), category_source: res.category_source.clone(),
            app: res.app.clone(), app_kind: res.app_kind.clone(), domain: res.domain.clone(), rule: res.rule.clone(), evidence: res.evidence.clone(),
        };
        if let Ok(Some(ovr)) = db.active_category_override(now, Some(&provisional)) {
            res.category = ovr.category; res.category_source = "manual".to_string();
        }
        res
    }

    fn resolve_window(&mut self, cfg: &ActivityConfig) -> FocusResolution {
        let Some(win) = query_active_window() else {
            return FocusResolution { category: "neutral".to_string(), category_source: "default".to_string(), source: "unattributed".to_string(), confidence: "low".to_string(), ..Default::default() };
        };
        resolve_focus(&win, cfg, &self.project_index, &mut self.firefox)
    }
}

pub fn explain_current(cfg: &ActivityConfig) -> Result<serde_json::Value, String> {
    ensure_hyprland_env();
    let idx = ProjectIndex::refresh();
    let mut firefox = FirefoxCache::default();
    let win = query_active_window().ok_or_else(|| "No active window (hyprctl activewindow failed)".to_string())?;
    let ctx = resolve_window_context(&win, cfg, &idx, &mut firefox);
    let mut ctx2 = ctx.clone();
    apply_rules(&mut ctx2, cfg, &idx);
    let mut hits: Vec<serde_json::Value> = Vec::new();
    for r in &cfg.presence.rules {
        if rule_matches(r, &ctx) { hits.push(serde_json::json!({"kind":"user","name":r.name,"category":r.category,"require":r.require})); }
    }
    for r in builtin_rules() {
        if rule_matches(&r, &ctx) { hits.push(serde_json::json!({"kind":"builtin","name":r.name,"category":r.category,"require":r.require})); }
    }
    Ok(serde_json::json!({
        "window": {"class": win.class, "initial_class": win.initial_class, "title": win.title, "pid": win.pid},
        "context": {"app": ctx2.app, "app_kind": ctx2.app_kind, "domain": ctx2.domain, "tmux_session": ctx2.tmux_session, "tmux_window": ctx2.tmux_window, "path": ctx2.path},
        "resolved": {"project_id": ctx2.project_id, "project_name": ctx2.project_name, "workspace": ctx2.workspace, "repo": ctx2.repo, "source": ctx2.source, "confidence": ctx2.confidence},
        "category": {"value": ctx2.category, "source": ctx2.category_source, "rule": ctx2.rule},
        "matches": hits,
    }))
}

pub fn attribute_path(path: &Path) -> Option<(String, String, String, String)> {
    let idx = ProjectIndex::refresh();
    idx.attribute_path(path)
}

pub fn rollup_focus(db: &ActivityDb, now_ms_val: i64) -> Result<(), String> {
    let last_rollup_hour = db.get_meta_i64("focus_rollup_hour").ok().flatten().unwrap_or(0);
    let from_hour = if last_rollup_hour > 0 { last_rollup_hour - 3600 } else { hour_floor(now_ms_val - 7 * 24 * 3600 * 1000) };
    let to_hour = hour_floor(now_ms_val) + 3600;
    let focus_intervals = db.focus_intervals_in_range(from_hour * 1000, to_hour * 1000)?;
    let presence_intervals = db.presence_intervals_in_range(from_hour * 1000, to_hour * 1000)?;
    let _ = db.conn.execute("DELETE FROM focus_buckets WHERE hour >= ?1 AND hour < ?2", rusqlite::params![from_hour, to_hour]);

    let mut hour = from_hour;
    while hour < to_hour {
        let hs = hour * 1000; let he = (hour + 3600) * 1000;
        let mut work: std::collections::HashMap<(String,String,String,String), i64> = std::collections::HashMap::new();
        let mut input: std::collections::HashMap<(String,String,String,String), i64> = std::collections::HashMap::new();
        let mut total_work=0i64; let mut total_active_work=0i64; let mut total_leisure=0i64; let mut total_neutral=0i64; let mut total_unattributed=0i64; let mut total_unassigned_work=0i64;
        for fi in &focus_intervals {
            let fstart = fi.start_ts.max(hs); let fend = fi.end_ts.unwrap_or(now_ms_val).min(he);
            if fend <= fstart { continue; }
            let present_secs = intersect_present_seconds(&presence_intervals, fstart, fend, now_ms_val);
            let active_secs = intersect_active_seconds(&presence_intervals, fstart, fend, now_ms_val);
            if present_secs <= 0 { continue; }
            match fi.category.as_str() {
                "work" => {
                    total_work += present_secs;
                    total_active_work += active_secs;
                    let key = (fi.project_id.clone(), fi.project_name.clone(), fi.workspace.clone(), fi.repo.clone());
                    if fi.project_id.is_empty() { total_unassigned_work += present_secs; }
                    else { *work.entry(key.clone()).or_insert(0) += present_secs; *input.entry(key).or_insert(0) += active_secs; }
                }
                "leisure" => { total_leisure += present_secs; }
                _ => { total_neutral += present_secs; if fi.project_id.is_empty() && fi.source == "unattributed" { total_unattributed += present_secs; } }
            }
        }
        for (key, secs) in &work {
            let inp = input.get(key).copied().unwrap_or(0);
            let _ = db.upsert_focus_bucket(&FocusBucket { project_id: key.0.clone(), project_name: key.1.clone(), hour, workspace: key.2.clone(), repo: key.3.clone(), work_seconds: *secs, input_seconds: inp });
        }
        if let Ok(mut buckets) = db.presence_buckets_in_range(hour, hour + 3600) {
            let mut b = buckets.pop().unwrap_or(super::model::PresenceBucket { hour, ..Default::default() });
            b.work_seconds = total_work; b.active_work_seconds = total_active_work; b.leisure_seconds = total_leisure; b.neutral_seconds = total_neutral;
            b.unattributed_seconds = total_unattributed; b.unassigned_work_seconds = total_unassigned_work;
            let _ = db.upsert_presence_bucket(&b);
        }
        hour += 3600;
    }
    let _ = db.set_meta("focus_rollup_hour", &hour_floor(now_ms_val).to_string());
    Ok(())
}

fn intersect_present_seconds(presence: &[super::model::PresenceInterval], start: i64, end: i64, now: i64) -> i64 {
    let mut secs = 0i64;
    for p in presence { if !matches!(p.state.as_str(), "active"|"idle") { continue; } let ps = p.start_ts.max(start); let pe = p.end_ts.unwrap_or(now).min(end); if pe > ps { secs += (pe-ps+999)/1000; } }
    secs.min(((end-start+999)/1000).max(0))
}
fn intersect_active_seconds(presence: &[super::model::PresenceInterval], start: i64, end: i64, now: i64) -> i64 {
    let mut secs = 0i64;
    for p in presence { if p.state.as_str() != "active" { continue; } let ps = p.start_ts.max(start); let pe = p.end_ts.unwrap_or(now).min(end); if pe > ps { secs += (pe-ps+999)/1000; } }
    secs.min(((end-start+999)/1000).max(0))
}
fn hour_floor(ts_ms: i64) -> i64 { if ts_ms <= 0 { return 0; } let secs = ts_ms/1000; (secs/3600)*3600 }

#[cfg(test)]
mod tests {
    use super::*;
    use crate::activity::config::{PresenceRule, PresenceRuleMatch, StringOrList};

    #[test]
    fn glob_match_basic() {
        assert!(glob_match("*firefox*", "Mozilla Firefox"));
        assert!(glob_match("mpv", "mpv"));
        assert!(glob_match("steam_app_*", "steam_app_1234"));
        assert!(!glob_match("firefox", "chromium"));
    }

    #[test]
    fn rule_matches_domain_and_class() {
        let ctx = WindowContext { app: "firefox".into(), app_kind: "browser".into(), app_class_raw: "firefox".into(), title: "YouTube".into(), domain: "youtube.com".into(), ..Default::default() };
        let rule = PresenceRule { name: Some("yt".into()), match_: PresenceRuleMatch { domain: StringOrList::One("youtube.com".into()), ..Default::default() }, category: Some("leisure".into()), project: None, workspace: None, repo: None, require: None };
        assert!(rule_matches(&rule, &ctx));
    }

    #[test]
    fn project_mapping_beats_leisure_rule() {
        let idx = ProjectIndex::default();
        let mut ctx = WindowContext { app: "firefox".into(), app_kind: "browser".into(), app_class_raw: "firefox".into(), title: "GitHub - repo".into(), domain: "github.com".into(), project_id: "p1".into(), project_name: "p1".into(), source: "cwd".into(), ..Default::default() };
        let cfg = ActivityConfig::default();
        apply_rules(&mut ctx, &cfg, &idx);
        assert_eq!(ctx.category, "work");
    }

    #[test]
    fn idle_browser_leisure_without_project() {
        let idx = ProjectIndex::default();
        let mut ctx = WindowContext { app: "firefox".into(), app_kind: "browser".into(), app_class_raw: "firefox".into(), title: "Instagram".into(), domain: "instagram.com".into(), ..Default::default() };
        let cfg = ActivityConfig::default();
        apply_rules(&mut ctx, &cfg, &idx);
        assert_eq!(ctx.category, "leisure");
    }

    #[test]
    fn terminal_without_project_is_neutral() {
        let idx = ProjectIndex::default();
        let mut ctx = WindowContext { app: "terminal".into(), app_kind: "terminal".into(), app_class_raw: "kitty".into(), title: "zsh".into(), ..Default::default() };
        let cfg = ActivityConfig::default();
        apply_rules(&mut ctx, &cfg, &idx);
        assert_eq!(ctx.category, "neutral");
    }
}
