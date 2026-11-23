use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk4::glib;
use gtk4::{Align, Box as GtkBox, Label, Orientation};
use libadwaita as adw;

use crate::charts;
use crate::data::{self, DashboardSnapshot, RangeKind};

pub struct DashboardWindow {
    pub window: adw::ApplicationWindow,
}

struct Widgets {
    range_dropdown: gtk4::DropDown,
    status_label: Label,
    cards_box: GtkBox,
    series_area: gtk4::DrawingArea,
    project_area: gtk4::DrawingArea,
    repo_area: gtk4::DrawingArea,
    details_label: Label,
    current_range: RefCell<RangeKind>,
}

impl DashboardWindow {
    pub fn new(app: &adw::Application) -> Self {
        let header = adw::HeaderBar::new();
        let title = adw::WindowTitle::new("WS Activity", "Local activity overview");
        header.set_title_widget(Some(&title));

        let range_model = gtk4::StringList::new(&["Today", "7 days", "30 days"]);
        let range_dropdown = gtk4::DropDown::new(Some(range_model), gtk4::Expression::NONE);
        range_dropdown.set_selected(0);
        let refresh_button = gtk4::Button::with_label("Refresh");
        header.pack_end(&refresh_button);
        header.pack_end(&range_dropdown);

        let status_label = Label::new(Some("Loading…"));
        status_label.set_halign(Align::Start);
        status_label.add_css_class("dim-label");
        status_label.set_margin_start(12);
        status_label.set_margin_end(12);
        status_label.set_margin_top(8);
        status_label.set_margin_bottom(4);

        let cards_box = GtkBox::new(Orientation::Horizontal, 8);
        cards_box.set_margin_start(12);
        cards_box.set_margin_end(12);
        cards_box.set_margin_top(4);
        cards_box.set_margin_bottom(8);
        cards_box.set_homogeneous(true);

        let series_title = section_label("Activity over time");
        let series_area = charts::timeseries::create_timeseries_chart();
        let series_card = card_box(&[series_title.upcast_ref(), series_area.upcast_ref()]);

        let project_title = section_label("Top projects");
        let project_area = charts::breakdown::create_breakdown_chart();
        let project_card = card_box(&[project_title.upcast_ref(), project_area.upcast_ref()]);

        let repo_title = section_label("Top repos/workspaces");
        let repo_area = charts::breakdown::create_breakdown_chart();
        let repo_card = card_box(&[repo_title.upcast_ref(), repo_area.upcast_ref()]);

        let breakdown_row = GtkBox::new(Orientation::Horizontal, 8);
        breakdown_row.set_margin_start(12);
        breakdown_row.set_margin_end(12);
        breakdown_row.set_margin_bottom(8);
        breakdown_row.set_homogeneous(true);
        breakdown_row.append(&project_card);
        breakdown_row.append(&repo_card);

        let details_label = Label::new(Some(""));
        details_label.set_halign(Align::Start);
        details_label.set_xalign(0.0);
        details_label.set_wrap(true);
        details_label.set_margin_start(12);
        details_label.set_margin_end(12);
        details_label.set_margin_bottom(12);
        let details_title = section_label("Status");
        let details_card = card_box(&[details_title.upcast_ref(), details_label.upcast_ref()]);

        let content = GtkBox::new(Orientation::Vertical, 0);
        content.append(&status_label);
        content.append(&cards_box);
        content.append(&series_card);
        content.append(&breakdown_row);
        content.append(&details_card);

        let scroll = gtk4::ScrolledWindow::builder()
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .vscrollbar_policy(gtk4::PolicyType::Automatic)
            .child(&content)
            .build();

        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&header);
        toolbar.set_content(Some(&scroll));

        let window = adw::ApplicationWindow::builder()
            .application(app)
            .title("WS Activity")
            .default_width(1100)
            .default_height(720)
            .content(&toolbar)
            .build();

        let widgets = Rc::new(Widgets {
            range_dropdown: range_dropdown.clone(),
            status_label,
            cards_box,
            series_area,
            project_area,
            repo_area,
            details_label,
            current_range: RefCell::new(RangeKind::Today),
        });

        {
            let w = widgets.clone();
            range_dropdown.connect_selected_notify(move |_| {
                request_refresh(&w);
            });
        }
        {
            let w = widgets.clone();
            refresh_button.connect_clicked(move |_| {
                request_refresh(&w);
            });
        }

        request_refresh(&widgets);

        Self { window }
    }

    pub fn present(&self) {
        self.window.present();
    }
}

fn request_refresh(w: &Rc<Widgets>) {
    w.status_label.set_text("Loading…");
    let range = match w.range_dropdown.selected() {
        1 => RangeKind::Week,
        2 => RangeKind::Month,
        _ => RangeKind::Today,
    };
    *w.current_range.borrow_mut() = range;

    let (sender, receiver) = async_channel::unbounded();
    std::thread::spawn(move || {
        let snapshot = data::load_snapshot(range, None);
        let _ = sender.send_blocking(snapshot);
    });

    let w = w.clone();
    glib::spawn_future_local(async move {
        if let Ok(snapshot) = receiver.recv().await {
            apply_snapshot(&w, snapshot);
        }
    });
}

fn apply_snapshot(w: &Widgets, snapshot: DashboardSnapshot) {
    if let Some(err) = snapshot.error {
        w.status_label.set_text(&format!("Error: {}", err));
        return;
    }

    let Some(status) = snapshot.status.as_ref() else {
        w.status_label.set_text("No status available");
        return;
    };

    let daemon = if status.daemon_running { "running" } else { "not running" };
    w.status_label.set_text(&format!(
        "Range: {}  •  Active: {}  •  Wall-clock: {}  •  Daemon: {}  •  Watching: {}/{}",
        w.current_range.borrow().label(),
        data::format_duration_human(snapshot.total_active_seconds),
        data::format_duration_human(snapshot.wall_clock_active_seconds),
        daemon,
        status.watching_workspaces,
        status.known_workspaces,
    ));

    // cards
    while let Some(child) = w.cards_box.first_child() { w.cards_box.remove(&child); }
    w.cards_box.append(&metric_card("Active", &data::format_duration_human(snapshot.total_active_seconds)));
    w.cards_box.append(&metric_card("Wall clock", &data::format_duration_human(snapshot.wall_clock_active_seconds)));
    let events = snapshot.rows.iter().map(|r| r.file_events + r.git_events + r.command_events).sum::<i64>();
    let sessions = snapshot.rows.iter().map(|r| r.session_count).sum::<i64>();
    w.cards_box.append(&metric_card("Events", &events.to_string()));
    w.cards_box.append(&metric_card("Sessions", &sessions.to_string()));
    w.cards_box.append(&metric_card("Daemon", if status.daemon_running { "running" } else { "stopped" }));

    let step = match *w.current_range.borrow() { RangeKind::Today => "hour", _ => "day" };
    charts::timeseries::draw_timeseries(&w.series_area, &snapshot.series, step);
    charts::breakdown::draw_breakdown(&w.project_area, &snapshot.project_breakdown, "Top projects");
    charts::breakdown::draw_breakdown(&w.repo_area, &snapshot.event_breakdown, "Top repos");

    let sync_errors = status.projects.iter().filter(|p| p.last_error.is_some() || p.backoff_until > ws_core::activity::db::now_ms()).count();
    let last_event = status.last_event_at.map(|ts| {
        let ago = (ws_core::activity::db::now_ms() - ts).max(0) / 1000;
        format!("{}s ago", ago)
    }).unwrap_or_else(|| "none".to_string());
    w.details_label.set_text(&format!(
        "Device: {}\nLast event: {}\nEvents: {}  Buckets: {}\nDB: {}\nSync errors/backoff: {}\nWatch limited: {}",
        status.device_name,
        last_event,
        status.event_count,
        status.bucket_count,
        status.db_path,
        sync_errors,
        status.watch_limited,
    ));
}

fn metric_card(title: &str, value: &str) -> gtk4::Frame {
    let b = GtkBox::new(Orientation::Vertical, 4);
    b.set_margin_start(8);
    b.set_margin_end(8);
    b.set_margin_top(8);
    b.set_margin_bottom(8);
    let t = Label::new(Some(title));
    t.add_css_class("dim-label");
    t.set_halign(Align::Start);
    let v = Label::new(Some(value));
    v.set_halign(Align::Start);
    v.add_css_class("title-2");
    b.append(&t);
    b.append(&v);
    let f = gtk4::Frame::new(None);
    f.set_child(Some(&b));
    f
}

fn section_label(text: &str) -> Label {
    let l = Label::new(Some(text));
    l.set_halign(Align::Start);
    l.add_css_class("heading");
    l.set_margin_start(12);
    l.set_margin_end(12);
    l.set_margin_top(8);
    l.set_margin_bottom(4);
    l
}

fn card_box(children: &[&gtk4::Widget]) -> gtk4::Frame {
    let b = GtkBox::new(Orientation::Vertical, 0);
    b.set_margin_top(4);
    b.set_margin_bottom(8);
    for c in children { b.append(*c); }
    let f = gtk4::Frame::new(None);
    f.set_margin_start(12);
    f.set_margin_end(12);
    f.set_margin_bottom(8);
    f.set_child(Some(&b));
    f
}
