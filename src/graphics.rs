/// Fast, dependency-free Base64 encoder
pub fn base64_encode(data: &[u8]) -> String {
    const CHARSET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::with_capacity((data.len() + 2) / 3 * 4);
    let mut i = 0;
    while i < data.len() {
        let chunk = &data[i..std::cmp::min(i + 3, data.len())];
        let val = match chunk.len() {
            3 => ((chunk[0] as u32) << 16) | ((chunk[1] as u32) << 8) | (chunk[2] as u32),
            2 => ((chunk[0] as u32) << 16) | ((chunk[1] as u32) << 8),
            1 => (chunk[0] as u32) << 16,
            _ => unreachable!(),
        };
        result.push(CHARSET[((val >> 18) & 63) as usize] as char);
        result.push(CHARSET[((val >> 12) & 63) as usize] as char);
        if chunk.len() >= 2 {
            result.push(CHARSET[((val >> 6) & 63) as usize] as char);
        } else {
            result.push('=');
        }
        if chunk.len() == 3 {
            result.push(CHARSET[(val & 63) as usize] as char);
        } else {
            result.push('=');
        }
        i += 3;
    }
    result
}

/// Generates the iTerm2 inline image sequence
pub fn make_iterm2_sequence(base64_data: &str, cols: u16, rows: u16) -> String {
    format!(
        "\x1b]1337;File=inline=1;width={};height={};preserveAspectRatio=1:{}\x07",
        cols, rows, base64_data
    )
}

/// Generates the Kitty graphics sequence (using quiet mode q=2 to avoid polluting stdin)
pub fn make_kitty_sequence(base64_data: &str, cols: u16, rows: u16) -> String {
    format!(
        "\x1b_Ga=T,f=100,t=d,c={},r={},q=2;{}\x1b\\",
        cols, rows, base64_data
    )
}

/// Generates the escape sequence to clear all Kitty graphics
pub fn make_kitty_clear_sequence() -> &'static str {
    "\x1b_Ga=d,d=a\x1b\\"
}

/// Checks if the current terminal supports the configured high-resolution graphics protocol
pub fn is_graphics_protocol_supported(renderer: &crate::config::ImageRenderer) -> bool {
    let term_prog = std::env::var("TERM_PROGRAM").unwrap_or_default();
    let term_type = std::env::var("TERM").unwrap_or_default();
    let has_kitty_id = std::env::var("KITTY_WINDOW_ID").is_ok();
    
    match renderer {
        crate::config::ImageRenderer::Iterm2 => {
            term_prog == "iTerm.app" || term_prog == "WezTerm"
        }
        crate::config::ImageRenderer::Kitty => {
            term_prog == "Ghostty" 
                || term_prog == "WezTerm" 
                || term_prog == "iTerm.app" 
                || has_kitty_id 
                || term_type.contains("kitty")
        }
        _ => true,
    }
}
