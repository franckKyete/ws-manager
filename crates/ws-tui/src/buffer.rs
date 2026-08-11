/// High-performance VT100 virtual terminal line buffer supporting true terminal screen grid,
/// in-place cursor overwriting, dynamic lossless pane resizing, horizontal panning, and immutable scrollback history.

use regex::Regex;
use vt100::Parser;

lazy_static::lazy_static! {
    pub static ref NON_COLOR_ANSI_REGEX: Regex = Regex::new(
        r"\x1b\[[0-9;?]*[A-LN-Za-ln-z]|\x1b\([AB012]|\x1b\][^\x07\x1b]*[\x07\x1b\\]"
    ).unwrap();
}

/// Sanitize terminal line: strip rogue non-color CSI cursor escapes while preserving ANSI SGR color formatting.
pub fn sanitize_terminal_line(line: &str) -> String {
    if line.is_empty() {
        return String::new();
    }
    let cleaned = NON_COLOR_ANSI_REGEX.replace_all(line, "");
    cleaned.trim_end_matches(&['\r', '\n'][..]).to_string()
}

pub struct VirtualLineBuffer {
    pub parser: Parser,
    pub max_lines: usize,
}

impl Default for VirtualLineBuffer {
    fn default() -> Self {
        Self::new(5000)
    }
}

impl VirtualLineBuffer {
    pub fn new(max_lines: usize) -> Self {
        Self {
            parser: Parser::new(24, 160, max_lines),
            max_lines,
        }
    }

    pub fn resize(&mut self, rows: u16, cols: u16) {
        if rows > 0 && cols > 0 {
            self.parser.set_size(rows, cols);
        }
    }

    pub fn feed_bytes(&mut self, data: &[u8]) {
        self.parser.process(data);
    }

    pub fn feed(&mut self, text: &str) {
        self.feed_bytes(text.as_bytes());
    }

    /// Return all visible screen lines as plain strings.
    pub fn get_lines(&self) -> Vec<String> {
        let screen = self.parser.screen();
        let contents = screen.contents();
        let mut lines: Vec<String> = contents.lines().map(|s| s.to_string()).collect();
        while lines.len() > 1 && lines.last().map(|l| l.trim().is_empty()).unwrap_or(false) {
            lines.pop();
        }
        lines
    }

    /// Return formatted ANSI rows for a viewport with vertical scrollback and horizontal panning.
    /// Returns `(rows, actual_scrollback_offset)`.
    pub fn get_formatted_rows(
        &mut self,
        scrollback_offset: usize,
        horizontal_offset: usize,
        height: usize,
        width: usize,
    ) -> (Vec<Vec<u8>>, usize) {
        if height > 0 && width > 0 {
            let cur_size = self.parser.screen().size();
            let target_cols = cur_size.1.max(width as u16).max(160);
            if cur_size.0 != height as u16 || cur_size.1 < target_cols {
                self.parser.set_size(height as u16, target_cols);
            }
        }

        self.parser.set_scrollback(scrollback_offset);
        let actual_offset = self.parser.screen().scrollback();
        let start_col = horizontal_offset as u16;
        let view_width = width as u16;
        let rows: Vec<Vec<u8>> = self
            .parser
            .screen()
            .rows_formatted(start_col, view_width)
            .take(height)
            .collect();
        self.parser.set_scrollback(0);
        (rows, actual_offset)
    }

    pub fn scrollback_count(&self) -> usize {
        self.parser.screen().scrollback()
    }

    pub fn clear(&mut self) {
        self.parser = Parser::new(24, 160, self.max_lines);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_in_place_carriage_return_progress() {
        let mut buf = VirtualLineBuffer::new(100);
        buf.feed("LOG Starting bundling...\r\n");
        buf.feed("\rAndroid entry.js 81.4%");
        buf.feed("\rAndroid entry.js 82.5%");
        buf.feed("\rAndroid entry.js 99.9%");
        buf.feed("\rAndroid Bundled 120ms\r\n");
        buf.feed("WARN Temporary warning\r\n");

        let lines = buf.get_lines();
        assert!(lines[0].contains("LOG Starting bundling..."));
        assert!(lines[1].contains("Android Bundled 120ms"));
        assert!(lines[2].contains("WARN Temporary warning"));
    }

    #[test]
    fn test_expo_qr_code_and_progress_preservation() {
        let mut buf = VirtualLineBuffer::new(100);
        buf.feed("Starting project at /workspace/develop/Renttik-mobile\r\n");
        buf.feed("Starting Metro Bundler\r\n");

        let sample_qr: &[&[u8]] = &[
            b"\x1b[47m  \x1b[40m                        \x1b[47m  \x1b[0m\r\n",
            b"\x1b[47m  \x1b[40m  \x1b[47m\xe2\x96\x88\xe2\x96\x88\xe2\x96\x88\xe2\x96\x88\xe2\x96\x88\xe2\x96\x88\xe2\x96\x88\x1b[40m  \x1b[47m\xe2\x96\x88\x1b[40m \x1b[47m\xe2\x96\x88\x1b[40m  \x1b[47m\xe2\x96\x88\xe2\x96\x88\xe2\x96\x88\xe2\x96\x88\xe2\x96\x88\xe2\x96\x88\xe2\x96\x88\x1b[40m  \x1b[47m  \x1b[0m\r\n",
            b"\x1b[47m  \x1b[40m  \x1b[47m\xe2\x96\x88\x1b[40m     \x1b[47m\xe2\x96\x88\x1b[40m  \x1b[47m\xe2\x96\x88\xe2\x96\x88\x1b[40m    \x1b[47m\xe2\x96\x88\x1b[40m     \x1b[47m\xe2\x96\x88\x1b[40m  \x1b[47m  \x1b[0m\r\n",
            b"\x1b[47m  \x1b[40m  \x1b[47m\xe2\x96\x88\xe2\x96\x88\xe2\x96\x88\xe2\x96\x88\xe2\x96\x88\xe2\x96\x88\xe2\x96\x88\x1b[40m  \x1b[47m\xe2\x96\x88\x1b[40m \x1b[47m\xe2\x96\x88\x1b[40m \x1b[47m\xe2\x96\x88\x1b[40m \x1b[47m\xe2\x96\x88\xe2\x96\x88\xe2\x96\x88\xe2\x96\x88\xe2\x96\x88\xe2\x96\x88\xe2\x96\x88\x1b[40m  \x1b[47m  \x1b[0m\r\n",
        ];

        for line in sample_qr {
            buf.feed_bytes(line);
        }

        buf.feed("› Press a │ open Android\r\n");
        buf.feed("› Press w │ open web\r\n\r\n");

        for pct in [81.4, 85.0, 90.0, 95.0, 97.7, 97.8, 99.9] {
            buf.feed_bytes(format!("\r\x1b[2KAndroid node_modules/expo-router/entry.js [=======   ] {}% (4894/4948)", pct).as_bytes());
        }

        buf.feed_bytes(b"\r\x1b[2KAndroid Bundled 471ms (4969 modules)\r\n");
        buf.feed_bytes(b"LOG Sentry disabled in development\r\n");

        let lines = buf.get_lines();
        assert!(lines[0].contains("Starting project"));
        assert!(lines[1].contains("Starting Metro Bundler"));
        assert!(lines[3].contains("███████")); // QR code preserved!
        assert!(lines.iter().any(|l| l.contains("Android Bundled 471ms")));
        assert!(lines.iter().any(|l| l.contains("LOG Sentry disabled")));
    }

    #[test]
    fn test_lossless_vertical_resizing_and_auto_following() {
        let mut buf = VirtualLineBuffer::new(5000);
        // Start with a 15-row pane
        buf.get_formatted_rows(0, 0, 15, 80);

        for i in 1..=30 {
            buf.feed(&format!("Log line {}\r\n", i));
        }

        // 1. Live view (offset 0): follows latest output down to Line 30
        let (rows_live, offset_0) = buf.get_formatted_rows(0, 0, 15, 80);
        assert_eq!(offset_0, 0);
        assert_eq!(rows_live.len(), 15);
        assert!(
            rows_live.iter().any(|r| String::from_utf8_lossy(r).contains("Log line 30")),
            "Live view must follow new output to Log line 30"
        );

        // 2. Shrink pane to 8 rows: top rows pushed to scrollback, latest output preserved
        let (rows_shrunk, _) = buf.get_formatted_rows(0, 0, 8, 80);
        assert_eq!(rows_shrunk.len(), 8);
        assert!(
            rows_shrunk.iter().any(|r| String::from_utf8_lossy(r).contains("Log line 30")),
            "Shrunk view must retain Log line 30"
        );

        // 3. Expand pane to 31 rows: pulls rows back from scrollback, all 30 lines restored!
        let (rows_expanded, _) = buf.get_formatted_rows(0, 0, 31, 80);
        assert_eq!(rows_expanded.len(), 31);
        let top_line = String::from_utf8_lossy(&rows_expanded[0]);
        let bot_line = String::from_utf8_lossy(&rows_expanded[29]);
        assert!(top_line.contains("Log line 1"), "Top of expanded pane must be Log line 1, got: {}", top_line);
        assert!(bot_line.contains("Log line 30"), "Bottom of expanded pane must be Log line 30, got: {}", bot_line);


        // 4. Scrollback navigation (scroll back 10 lines): shows historical lines without panics
        let (rows_scrolled, actual_offset) = buf.get_formatted_rows(10, 0, 15, 80);
        assert!(actual_offset > 0, "Actual scrollback offset must be > 0");
        assert_eq!(rows_scrolled.len(), 15);
    }

    #[test]
    fn test_lossless_resizing_and_horizontal_panning() {
        let mut buf = VirtualLineBuffer::new(5000);
        let very_long_line = "https://developer.android.com/studio/run/device.html#developer-device-options. If you are using Genymotion go to Settings -> ADB, select custom SDK directory.";
        buf.feed(&format!("{}\r\n", very_long_line));

        // 1. Narrow viewport (40 cols) starting at col 0
        let (rows_col0, _) = buf.get_formatted_rows(0, 0, 10, 40);
        let text_col0 = String::from_utf8_lossy(&rows_col0[0]);
        assert!(text_col0.starts_with("https://developer.android.com/studio/"));

        // 2. Horizontal scroll to col 20 (starts at "droid.com")
        let (rows_col20, _) = buf.get_formatted_rows(0, 20, 10, 40);
        let text_col20 = String::from_utf8_lossy(&rows_col20[0]);
        assert!(text_col20.contains("droid.com"));

        // 3. Widen viewport back to 160 cols -> all text is 100% intact without any loss!
        let (rows_wide, _) = buf.get_formatted_rows(0, 0, 10, 160);
        let text_wide = String::from_utf8_lossy(&rows_wide[0]);
        assert!(text_wide.contains("Settings -> ADB, select custom SDK directory."));
    }
}
