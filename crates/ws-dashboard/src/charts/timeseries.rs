use gtk4::prelude::*;
use gtk4::DrawingArea;
use plotters::prelude::*;
use plotters_cairo::CairoBackend;

use ws_core::activity::model::ActivitySeriesPoint;

pub fn create_timeseries_chart() -> DrawingArea {
    let area = DrawingArea::new();
    area.set_content_height(260);
    area.set_hexpand(true);
    area.set_vexpand(false);
    area
}

pub fn draw_timeseries(area: &DrawingArea, series: &[ActivitySeriesPoint], step: &str) {
    let series = series.to_vec();
    let step = step.to_string();
    area.set_draw_func(move |_area, cr, width, height| {
        if width <= 10 || height <= 10 { return; }
        let backend = CairoBackend::new(cr, (width as u32, height as u32)).ok();
        let Some(backend) = backend else { return; };
        let root = backend.into_drawing_area();
        let _ = root.fill(&WHITE);

        if series.is_empty() {
            let _ = root.draw_text(
                "No activity in selected range",
                &("sans-serif", 14).into_font().color(&RGBColor(120, 120, 120)),
                (width / 2 - 90, height / 2),
            );
            let _ = root.present();
            return;
        }

        let max_val = series.iter().map(|p| p.active_seconds).max().unwrap_or(1).max(1);
        let x_max = series.len().max(1);
        let y_max = ((max_val as f64 * 1.15) / 60.0).ceil().max(1.0) as i32; // minutes

        let mut chart = match ChartBuilder::on(&root)
            .margin(10)
            .x_label_area_size(30)
            .y_label_area_size(45)
            .build_cartesian_2d(0i32..(x_max as i32), 0i32..y_max)
        {
            Ok(c) => c,
            Err(_) => { let _ = root.present(); return; }
        };

        let _ = chart.configure_mesh()
            .disable_x_mesh()
            .y_desc("Active minutes")
            .label_style(("sans-serif", 12))
            .axis_style(RGBColor(180, 180, 180))
            .draw();

        let bar_style = RGBColor(66, 133, 244).filled();
        let _ = chart.draw_series(series.iter().enumerate().map(|(i, p)| {
            let y = (p.active_seconds as f64 / 60.0).round() as i32;
            Rectangle::new([(i as i32, 0), (i as i32 + 1, y.max(0))], bar_style)
        }));

        // simple x labels: first/middle/last
        let labels = [0usize, x_max / 2, x_max.saturating_sub(1)];
        for idx in labels {
            if let Some(p) = series.get(idx) {
                let text = format_bucket_label(p.bucket_start, &step);
                let x = (width as f64) * ((idx as f64 + 0.5) / x_max as f64);
                let _ = root.draw_text(
                    &text,
                    &("sans-serif", 10).into_font().color(&RGBColor(100, 100, 100)),
                    (x as i32 - 20, height - 18),
                );
            }
        }

        let _ = root.present();
    });
}

fn format_bucket_label(epoch_secs: i64, step: &str) -> String {
    use chrono::{TimeZone, Utc};
    let dt = Utc.timestamp_opt(epoch_secs, 0).single();
    match dt {
        Some(dt) => {
            let local = dt.with_timezone(&chrono::Local);
            if step == "day" { local.format("%m-%d").to_string() } else { local.format("%H:%M").to_string() }
        }
        None => "".to_string(),
    }
}
