use std::sync::mpsc;
use std::thread;

use wayland_client::protocol::{wl_registry, wl_seat};
use wayland_client::{delegate_noop, Connection, Dispatch, QueueHandle};
use wayland_protocols::ext::idle_notify::v1::client::ext_idle_notification_v1;
use wayland_protocols::ext::idle_notify::v1::client::ext_idle_notifier_v1;

use super::config::ActivityPresenceConfig;
use super::presence::PresenceEvent;

pub fn spawn_wayland_watcher(cfg: &ActivityPresenceConfig) -> Option<mpsc::Receiver<PresenceEvent>> {
    let (tx, rx) = mpsc::channel();
    let idle_timeout_ms = (cfg.idle_timeout.max(30) * 1000) as u32;

    let handle = thread::Builder::new()
        .name("ws-presence-wayland".to_string())
        .spawn(move || {
            if let Err(e) = run(tx, idle_timeout_ms) {
                eprintln!("[activity/presence] wayland idle watcher stopped: {e}");
            }
        });
    if handle.is_err() {
        return None;
    }
    Some(rx)
}

struct State {
    tx: mpsc::Sender<PresenceEvent>,
    idle_timeout_ms: u32,
    seat: Option<wl_seat::WlSeat>,
    notifier: Option<ext_idle_notifier_v1::ExtIdleNotifierV1>,
    notifier_version: u32,
    notification: Option<ext_idle_notification_v1::ExtIdleNotificationV1>,
}

fn run(tx: mpsc::Sender<PresenceEvent>, idle_timeout_ms: u32) -> Result<(), String> {
    // Try connecting; discover WAYLAND_DISPLAY if missing
    if std::env::var("WAYLAND_DISPLAY").is_err() {
        if let Some(display) = discover_wayland_display() {
            std::env::set_var("WAYLAND_DISPLAY", display);
        }
    }
    let conn = Connection::connect_to_env().map_err(|e| format!("connect: {e}"))?;
    let mut event_queue = conn.new_event_queue();
    let qh = event_queue.handle();
    let display = conn.display();
    display.get_registry(&qh, ());

    let mut state = State {
        tx,
        idle_timeout_ms,
        seat: None,
        notifier: None,
        notifier_version: 0,
        notification: None,
    };

    // Initial roundtrip to get globals
    event_queue.roundtrip(&mut state).map_err(|e| format!("roundtrip: {e}"))?;

    // Create notification if possible
    setup_notification(&qh, &mut state);

    loop {
        event_queue.blocking_dispatch(&mut state).map_err(|e| format!("dispatch: {e}"))?;
    }
}

fn discover_wayland_display() -> Option<String> {
    let runtime = std::env::var("XDG_RUNTIME_DIR").ok()?;
    let dir = std::fs::read_dir(&runtime).ok()?;
    for entry in dir.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with("wayland-") && !name.ends_with(".lock") {
            // Prefer wayland-1 style sockets
            let path = entry.path();
            if path.is_dir() { continue; }
            return Some(name);
        }
    }
    None
}

fn setup_notification(qh: &QueueHandle<State>, state: &mut State) {
    if state.notification.is_some() { return; }
    let (Some(seat), Some(notifier)) = (state.seat.as_ref(), state.notifier.as_ref()) else { return; };
    let notification = if state.notifier_version >= 2 {
        notifier.get_input_idle_notification(state.idle_timeout_ms, seat, qh, ())
    } else {
        notifier.get_idle_notification(state.idle_timeout_ms, seat, qh, ())
    };
    state.notification = Some(notification);
}

impl Dispatch<wl_registry::WlRegistry, ()> for State {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global { name, interface, version } = event {
            match interface.as_str() {
                "wl_seat" => {
                    if state.seat.is_none() {
                        let seat = registry.bind::<wl_seat::WlSeat, _, _>(name, version.min(7), qh, ());
                        state.seat = Some(seat);
                        setup_notification(qh, state);
                    }
                }
                "ext_idle_notifier_v1" => {
                    if state.notifier.is_none() {
                        let ver = version.min(2);
                        let notifier = registry.bind::<ext_idle_notifier_v1::ExtIdleNotifierV1, _, _>(name, ver, qh, ());
                        state.notifier = Some(notifier);
                        state.notifier_version = ver;
                        setup_notification(qh, state);
                    }
                }
                _ => {}
            }
        }
    }
}

impl Dispatch<ext_idle_notifier_v1::ExtIdleNotifierV1, ()> for State {
    fn event(_: &mut Self, _: &ext_idle_notifier_v1::ExtIdleNotifierV1, _: ext_idle_notifier_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<ext_idle_notification_v1::ExtIdleNotificationV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &ext_idle_notification_v1::ExtIdleNotificationV1,
        event: ext_idle_notification_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let now = super::db::now_ms();
        match event {
            ext_idle_notification_v1::Event::Idled => {
                // Last input was approximately now - idle_timeout
                let last_input = now - (state.idle_timeout_ms as i64);
                let _ = state.tx.send(PresenceEvent::Idled { ts_ms: last_input });
            }
            ext_idle_notification_v1::Event::Resumed => {
                let _ = state.tx.send(PresenceEvent::Resumed { ts_ms: now });
            }
            _ => {}
        }
    }
}

delegate_noop!(State: ignore wl_seat::WlSeat);
