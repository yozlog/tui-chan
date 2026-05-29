use std::{env, fs, io, path::Path};

/// Previews layout mode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageLayout {
    Inline,
    Split,
    Hybrid,
}

impl ImageLayout {
    pub fn from_str(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "split" => ImageLayout::Split,
            "hybrid" => ImageLayout::Hybrid,
            _ => ImageLayout::Inline,
        }
    }

    #[allow(dead_code)]
    pub fn to_str(&self) -> &'static str {
        match self {
            ImageLayout::Inline => "inline",
            ImageLayout::Split => "split",
            ImageLayout::Hybrid => "hybrid",
        }
    }
}

/// Image renderer protocol
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageRenderer {
    Unicode,
    Iterm2,
    Kitty,
}

impl ImageRenderer {
    pub fn from_str(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "iterm2" => ImageRenderer::Iterm2,
            "kitty" => ImageRenderer::Kitty,
            _ => ImageRenderer::Unicode,
        }
    }

    #[allow(dead_code)]
    pub fn to_str(&self) -> &'static str {
        match self {
            ImageRenderer::Unicode => "unicode",
            ImageRenderer::Iterm2 => "iterm2",
            ImageRenderer::Kitty => "kitty",
        }
    }
}

/// Application configuration structure
#[derive(Debug, Clone)]
pub struct Config {
    /// Toggle whether images are rendered in the terminal
    pub render_images: bool,
    /// Preview layout mode: inline, split
    pub image_layout: ImageLayout,
    /// Image rendering backend
    pub image_renderer: ImageRenderer,
    /// Toggle line numbers in the board list
    pub board_line_numbers: bool,
    /// Toggle relative line numbers in the board list
    pub board_relative_line_numbers: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            render_images: false, // Default in code is false
            image_layout: ImageLayout::Inline, // Default layout mode is inline
            image_renderer: ImageRenderer::Unicode,
            board_line_numbers: false,
            board_relative_line_numbers: false,
        }
    }
}

impl Config {
    /// Parse configuration options from key-value file contents
    pub fn parse_from_file(contents: &str) -> Self {
        let mut config = Config::default();
        for line in contents.lines() {
            let line = line.trim();
            // Skip empty lines and comments
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some((key, val)) = line.split_once('=') {
                match key.trim() {
                    "render_images" => {
                        if let Ok(b) = val.trim().parse::<bool>() {
                            config.render_images = b;
                        }
                    }
                    "image_layout" => {
                        config.image_layout = ImageLayout::from_str(val.trim());
                    }
                    "image_renderer" => {
                        config.image_renderer = ImageRenderer::from_str(val.trim());
                    }
                    "board_line_numbers" => {
                        if let Ok(b) = val.trim().parse::<bool>() {
                            config.board_line_numbers = b;
                        }
                    }
                    "board_relative_line_numbers" => {
                        if let Ok(b) = val.trim().parse::<bool>() {
                            config.board_relative_line_numbers = b;
                        }
                    }
                    _ => {}
                }
            }
        }
        config
    }

    /// Generate default configuration file content
    pub fn default_file_contents() -> String {
        let mut contents = String::new();
        contents.push_str("# Settings for tui-chan\n\n");
        contents.push_str("# Enable/disable image rendering in the terminal using half-blocks\n");
        contents.push_str("render_images=false\n\n");
        contents.push_str("# Image preview layout mode in the terminal (only used if render_images is true)\n");
        contents.push_str("# Allowed values:\n");
        contents.push_str("#   inline - 4chan-style left thumbnail next to post text\n");
        contents.push_str("#   split  - splits active panel horizontally with large preview on right\n");
        contents.push_str("#   hybrid - show both inline thumbnails and split preview simultaneously\n");
        contents.push_str("image_layout=inline\n\n");
        contents.push_str("# Image renderer protocol (only used if render_images is true)\n");
        contents.push_str("# Allowed values:\n");
        contents.push_str("#   unicode - pre-rendered Unicode half-blocks (▄)\n");
        contents.push_str("#   iterm2  - iTerm2 inline image protocol (high resolution)\n");
        contents.push_str("#   kitty   - Kitty graphics protocol (high resolution)\n");
        contents.push_str("image_renderer=unicode\n\n");
        contents.push_str("# Enable/disable line numbers in the board list\n");
        contents.push_str("board_line_numbers=false\n\n");
        contents.push_str("# Enable/disable relative line numbers in the board list (only used if board_line_numbers is true)\n");
        contents.push_str("board_relative_line_numbers=false\n");
        contents
    }
}

/// Read the config file `settings.conf` or create one with default values if it doesn't exist
pub fn read_or_create_config_file() -> Result<Config, io::Error> {
    let Ok(config_dir) = env::var("XDG_CONFIG_HOME")
        .or_else(|_| env::var("HOME").map(|home| format!("{}/.config", home))) else {
        return Ok(Config::default());
    };

    let folder = format!("{config_dir}/tui-chan");
    let filepath = format!("{folder}/settings.conf");

    // Create the directory if it doesn't exist
    if !Path::new(&folder).exists() {
        fs::create_dir_all(&folder)?;
    }

    // Create the file with default values if it doesn't exist
    if !Path::new(&filepath).exists() {
        let default_contents = Config::default_file_contents();
        fs::write(&filepath, &default_contents)?;
        return Ok(Config::default());
    }

    // Read the file and parse config
    let contents = fs::read_to_string(&filepath)?;
    let config = Config::parse_from_file(&contents);

    Ok(config)
}
