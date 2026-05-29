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
}

/// Backend for board searching
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoardSearchBackend {
    Native,
    External,
}

impl BoardSearchBackend {
    pub fn from_str(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "external" => BoardSearchBackend::External,
            _ => BoardSearchBackend::Native,
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
    /// Toggle fzf board searching
    pub fzf_board_search: bool,
    /// Backend for board search
    pub board_search_backend: BoardSearchBackend,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            render_images: false, // Default in code is false
            image_layout: ImageLayout::Inline, // Default layout mode is inline
            image_renderer: ImageRenderer::Unicode,
            board_line_numbers: false,
            board_relative_line_numbers: false,
            fzf_board_search: false,
            board_search_backend: BoardSearchBackend::Native,
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
                    "fzf_board_search" => {
                        if let Ok(b) = val.trim().parse::<bool>() {
                            config.fzf_board_search = b;
                        }
                    }
                    "board_search_backend" => {
                        config.board_search_backend = BoardSearchBackend::from_str(val.trim());
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
        contents.push_str("# Show line numbers in the board list\n");
        contents.push_str("# Useful for Vim-style navigation: prefix a move key with a number (e.g. 3j, 5k)\n");
        contents.push_str("# to jump multiple rows at once. If you remap wasd -> hjkl, the same applies.\n");
        contents.push_str("board_line_numbers=false\n\n");
        contents.push_str("# Show relative line numbers instead of absolute ones (requires board_line_numbers=true)\n");
        contents.push_str("# The selected board is always 0; rows above and below are numbered by distance.\n");
        contents.push_str("# Makes it easy to see at a glance how many steps to reach any board.\n");
        contents.push_str("board_relative_line_numbers=false\n\n");
        contents.push_str("# Toggle fzf search for boards\n");
        contents.push_str("fzf_board_search=false\n\n");
        contents.push_str("# Backend for board search (native or external)\n");
        contents.push_str("# Note: using 'external' requires installing the 'fzf' CLI tool on your system.\n");
        contents.push_str("board_search_backend=native\n");
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
