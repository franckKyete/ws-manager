use gtk4::prelude::*;
use gtk4::DrawingArea;

use ws_core::activity::model::ActivityBreakdownRow;

pub fn create_breakdown_chart() -> DrawingArea {
    let area = DrawingArea::new();
    area.set_content_height(220);
    area.set_hexpand(true);
    area.set_vexpand(false);
    area
}

pub fn draw_breakdown(area: &DrawingArea, rows: &[ActivityBreakdownRow], title: &str) {
    let rows: Vec<ActivityBreakdownRow> = rows.iter().take(8).cloned().collect();
    let title = title.to_string();
    area.set_draw_func(move |_area, cr, width, height| {
        if width <= 10 || height <= 10 {
            return;
        }
        // background
        cr.set_source_rgb(1.0, 1.0, 1.0);
        let _ = cr.paint();

        cr.select_font_face("Sans", cairo::FontSlant::Normal, cairo::FontWeight::Bold);
        cr.set_font_size(13.0);
        cr.set_source_rgb(0.2, 0.2, 0.2);
        cr.move_to(10.0, 20.0);
        let _ = cr.show_text(&title);

        if rows.is_empty() {
            cr.select_font_face("Sans", cairo::FontSlant::Normal, cairo::FontWeight::Normal);
            cr.set_font_size(12.0);
            cr.set_source_rgb(0.5, 0.5, 0.5);
            cr.move_to(10.0, 50.0);
            let _ = cr.show_text("No data");
            return;
        }

        let max_val = rows
            .iter()
            .map(|r| r.active_seconds)
            .max()
            .unwrap_or(1)
            .max(1) as f64;
        let left = 140.0f64;
        let right_pad = 20.0f64;
        let bar_w_max = (width as f64 - left - right_pad).max(20.0);
        let top = 35.0f64;
        let row_h = 20.0f64;

        for (i, row) in rows.iter().enumerate() {
            let y = top + (i as f64) * row_h;
            let label = if row.key.chars().count() > 18 {
                let mut s: String = row.key.chars().take(17).collect();
                s.push('…');
                s
            } else {
                row.key.clone()
            };
            cr.select_font_face("Sans", cairo::FontSlant::Normal, cairo::FontWeight::Normal);
            cr.set_font_size(11.0);
            cr.set_source_rgb(0.25, 0.25, 0.25);
            cr.move_to(10.0, y + 12.0);
            let _ = cr.show_text(&label);

            let frac = (row.active_seconds as f64 / max_val).clamp(0.0, 1.0);
            let bw = (bar_w_max * frac).max(1.0);
            cr.set_source_rgb(0.26, 0.52, 0.96);
            cr.rectangle(left, y + 2.0, bw, 12.0);
            let _ = cr.fill();

            let mins = row.active_seconds / 60;
            let txt = if mins >= 60 {
                format!("{}h {}m", mins / 60, mins % 60)
            } else {
                format!("{}m", mins)
            };
            cr.set_source_rgb(0.3, 0.3, 0.3);
            cr.move_to(left + bw + 6.0, y + 12.0);
            let _ = cr.show_text(&txt);
        }
    });
}
