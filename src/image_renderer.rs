use image::{DynamicImage, GenericImageView};
use tui::style::{Color, Style};
use tui::text::{Span, Spans};

/// Converts an image to a sequence of `tui::text::Spans` using Unicode `▄` (half-block) characters.
/// Keeps aspect ratio using the `thumbnail` function from `image` crate.
pub fn render_half_blocks(img: &DynamicImage, max_w: u16, max_h: u16) -> Vec<Spans<'static>> {
    // If the box is 0, return empty list
    if max_w == 0 || max_h == 0 {
        return Vec::new();
    }

    // Each terminal cell holds 2 vertical pixels.
    // So the pixel boundaries we fit into are max_w horizontally, and max_h * 2 vertically.
    let max_pixel_w = max_w as u32;
    let max_pixel_h = (max_h as u32) * 2;

    // Scale down to fit inside bounds while preserving aspect ratio.
    let resized = img.thumbnail(max_pixel_w, max_pixel_h);
    let (w, h) = resized.dimensions();

    let mut lines = Vec::new();
    // Step by 2 vertically since each terminal row handles 2 vertical pixels (top and bottom)
    for y in (0..h).step_by(2) {
        let mut row = Vec::new();
        for x in 0..w {
            let top_pixel = resized.get_pixel(x, y);
            let bottom_pixel = if y + 1 < h {
                resized.get_pixel(x, y + 1)
            } else {
                // If odd height, bottom half of the last cell is black
                image::Rgba([0, 0, 0, 255])
            };

            let fg = Color::Rgb(bottom_pixel[0], bottom_pixel[1], bottom_pixel[2]);
            let bg = Color::Rgb(top_pixel[0], top_pixel[1], top_pixel[2]);

            row.push(Span::styled("▄", Style::default().fg(fg).bg(bg)));
        }
        lines.push(Spans::from(row));
    }

    lines
}
