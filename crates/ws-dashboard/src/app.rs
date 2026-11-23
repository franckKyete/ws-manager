use adw::prelude::*;
use libadwaita as adw;

use crate::window::DashboardWindow;

pub const APP_ID: &str = "dev.ws.activity.dashboard";

pub fn run() -> glib::ExitCode {
    let app = adw::Application::builder().application_id(APP_ID).build();
    app.connect_activate(build_ui);
    app.run()
}

fn build_ui(app: &adw::Application) {
    if let Some(window) = app.active_window() {
        window.present();
        return;
    }
    let window = DashboardWindow::new(app);
    window.present();
}
