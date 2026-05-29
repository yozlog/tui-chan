#![allow(clippy::single_match)]

use std::{env, io, process, str};

use client::ChanClient;
use clipboard::{ClipboardContext, ClipboardProvider};
use open::that as open_in_browser;
use reqwest::Client;
use termion::input::MouseTerminal;
use termion::raw::IntoRawMode;
use termion::screen::AlternateScreen;
use tokio::runtime::Runtime;
use tui::backend::TermionBackend;
use tui::layout::{Constraint, Direction, Layout, Rect};
use tui::style::{Color, Modifier, Style};
use tui::text::{Span, Spans};
use tui::widgets::{Block, Borders, List, ListItem, Paragraph, Wrap};
use tui::Terminal;

use crate::app::App;
use crate::client::api::{
    from_name as channel_provider_from_name, ChannelProvider, ContentUrlProvider,
};
use crate::event::{Event, Events};
use crate::format::{format_default, format_post_full, format_post_short};
use crate::keybinds::{read_or_create_keybinds_file, Keybinds};
use crate::model::{Board, Thread, ThreadList, ThreadPost};
use crate::style::{SelectedField, StyleProvider};

mod app;
mod client;
mod config;
mod event;
mod format;
mod graphics;
mod image_cache;
mod image_renderer;
mod keybinds;
mod model;
mod style;

use crate::config::read_or_create_config_file;
use crate::image_cache::ImageCache;


fn main() -> Result<(), io::Error> {
    // Get keybinds from config file
    let keybinds = read_or_create_keybinds_file().expect("Failed to read keybinds file");
    let keybinds = Keybinds::parse_from_file(&keybinds).expect("Failed to parse keybinds file");

    // Load settings from settings.conf file
    let mut config = read_or_create_config_file().expect("Failed to read config file");


    let stdout = io::stdout().into_raw_mode()?;
    let stdout = MouseTerminal::from(stdout);
    let stdout = AlternateScreen::from(stdout);
    let backend = TermionBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    let runtime = Runtime::new()?;
    let tokio_handle = runtime.handle().clone();
    let events = Events::new();
    let image_cache = ImageCache::new(tokio_handle, events.tx());

    let args: Vec<String> = env::args().collect();
    let chan: &str = if args.len() == 1 { "default" } else { &args[1] };

    let api: &dyn ChannelProvider = match channel_provider_from_name(chan) {
        Some(api) => api,
        None => {
            println!("Imageboard name \"{}\" is not valid.", chan);
            process::exit(1);
        }
    };

    let client = ChanClient::new(Client::new(), api.as_api());
    let api: &dyn ContentUrlProvider = api.as_content();

    let mut boards: Vec<Board> = vec![];
    runtime.block_on(async {
        let result = client.get_boards().await;

        match result {
            Ok(data) => boards = data,
            Err(_) => panic!("Could not fetch boards"),
        };
    });

    let mut app = App::new(boards, vec![], vec![], &keybinds);
    app.set_shown_board_list(true);
    let mut selected_field: SelectedField = SelectedField::BoardList;
    let mut thread_list = ThreadList::new();
    let style_prov = StyleProvider::new();
    let mut ctx: ClipboardContext = ClipboardProvider::new().unwrap();
    let mut last_selected_field = selected_field;
    let mut last_screen_share = app.calc_screen_share();
    let mut last_image_area: Option<tui::layout::Rect> = None;
    let mut last_image_url: Option<String> = None;
    let mut vim_prefix: u32 = 0;
    let mut has_vim_prefix = false;

    loop {
        let scr_share = app.calc_screen_share();
        let layout_changed = last_selected_field != selected_field || last_screen_share != scr_share;
        
        if layout_changed {
            if let Some(area) = last_image_area {
                if config.render_images {
                    if config.image_renderer == crate::config::ImageRenderer::Kitty {
                        print!("{}", crate::graphics::make_kitty_clear_sequence());
                        let _ = io::Write::flush(&mut io::stdout());
                    } else if config.image_renderer == crate::config::ImageRenderer::Iterm2 {
                        // Manually print spaces to stdout over the old image cells to bypass tui-rs virtual diffing
                        for r in 0..area.height {
                            print!("\x1b[{};{}H{}", area.y + r + 1, area.x + 1, " ".repeat(area.width as usize));
                        }
                        let _ = io::Write::flush(&mut io::stdout());
                    }
                }
                let _ = terminal.clear();
                last_image_area = None;
                last_image_url = None;
            }
            last_selected_field = selected_field;
            last_screen_share = scr_share;
        }

        if config.render_images {
            let prefetch_url = match selected_field {
                SelectedField::BoardList => None,
                SelectedField::ThreadList => app.media_url_threads(api),
                SelectedField::Thread => app.media_url_thread(api),
            };
            if let Some(url) = prefetch_url {
                image_cache.get_image(&url, true);
            }
        }

        let mut active_image_url: Option<String> = None;
        let mut active_image_area: Option<tui::layout::Rect> = None;
        let mut last_layout_chunk_1 = tui::layout::Rect::default();
        let mut last_layout_chunk_2 = tui::layout::Rect::default();

        let current_image_url = if config.render_images && (config.image_layout == crate::config::ImageLayout::Split || config.image_layout == crate::config::ImageLayout::Hybrid) {
            match selected_field {
                SelectedField::BoardList => None,
                SelectedField::ThreadList => app.media_url_threads(api),
                SelectedField::Thread => app.media_url_thread(api),
            }
        } else {
            None
        };

        let url_changed = match (&current_image_url, &last_image_url) {
            (Some(act), Some(lst)) => act != lst,
            (None, Some(_)) => true,
            _ => false,
        };

        if url_changed {
            if config.render_images && config.image_renderer != crate::config::ImageRenderer::Unicode {
                let is_supported = crate::graphics::is_graphics_protocol_supported(&config.image_renderer);

                if current_image_url.is_none() && last_image_url.is_some() {
                    // Manually print spaces to erase the entire preview block (including border lines) 
                    // from the screen, BEFORE terminal.draw is called. This guarantees the old borders 
                    // are erased first, and then the text can occupy the full width cleanly.
                    let target_chunk = match selected_field {
                        SelectedField::Thread => last_layout_chunk_2,
                        _ => last_layout_chunk_1,
                    };
                    let split = Layout::default()
                        .direction(Direction::Horizontal)
                        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)].as_ref())
                        .split(target_chunk);
                    let prev_box = split[1];
                    for r in 0..prev_box.height {
                        print!("\x1b[{};{}H{}", prev_box.y + r + 1, prev_box.x + 1, " ".repeat(prev_box.width as usize));
                    }
                    let _ = io::Write::flush(&mut io::stdout());
                }

                if let Some(area) = last_image_area {
                    if config.image_renderer == crate::config::ImageRenderer::Kitty && is_supported {
                        print!("{}", crate::graphics::make_kitty_clear_sequence());
                        let _ = io::Write::flush(&mut io::stdout());
                    } else if config.image_renderer == crate::config::ImageRenderer::Iterm2 && is_supported {
                        // Manually print spaces to stdout over the old image cells to bypass tui-rs virtual diffing
                        for r in 0..area.height {
                            print!("\x1b[{};{}H{}", area.y + r + 1, area.x + 1, " ".repeat(area.width as usize));
                        }
                        let _ = io::Write::flush(&mut io::stdout());
                    }
                }
            }
            last_image_area = None;
            last_image_url = None;
        }

        terminal.draw(|f| {
            let block_style = style_prov.default_from_selected_field(&selected_field);
            let scr_share = app.calc_screen_share();

            let mut constraints = vec![Constraint::Min(0)];
            if app.help_bar().shown() {
                constraints.push(Constraint::Length(10));
            }

            let helpbar_chunk = Layout::default()
                .constraints::<&[Constraint]>(constraints.as_ref())
                .split(f.size());

            if app.help_bar().shown() {
                let block = Block::default().borders(Borders::NONE).title(Span::styled(
                    app.help_bar().title(),
                    Style::default()
                        .fg(Color::Magenta)
                        .add_modifier(Modifier::BOLD),
                ));
                let paragraph = Paragraph::new(app.help_bar().text().as_str())
                    .block(block)
                    .wrap(Wrap { trim: true });
                f.render_widget(paragraph, helpbar_chunk[1]);
            }

            let chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints(
                    [
                        Constraint::Percentage(scr_share.board_list()),
                        Constraint::Percentage(scr_share.thread_list()),
                        Constraint::Percentage(scr_share.thread()),
                    ]
                    .as_ref(),
                )
                .split(helpbar_chunk[0]);

            let items: Vec<ListItem> = app
                .boards
                .items
                .iter()
                .enumerate()
                .map(|(i, board)| {
                    let mut spans = vec![];

                    if config.board_line_numbers {
                        let selected_idx = app.boards.state.selected().unwrap_or(0);
                        let max_num = if config.board_relative_line_numbers {
                            app.boards.items.len().saturating_sub(1)
                        } else {
                            app.boards.items.len()
                        };
                        let num_width = if max_num >= 100 { 3 } else if max_num >= 10 { 2 } else { 1 };
                        let num = if config.board_relative_line_numbers {
                            (i as isize - selected_idx as isize).abs()
                        } else {
                            (i + 1) as isize
                        };
                        let num_str = format!("{:>width$} ", num, width = num_width);
                        spans.push(Span::styled(
                            num_str,
                            Style::default().fg(Color::Indexed(242)),
                        ));
                    }

                    spans.push(Span::styled(
                        if config.board_line_numbers {
                            format!("/{}/", board.board())
                        } else {
                            format_default(&format!("/{}/", board.board()))
                        },
                        Style::default().fg(Color::Magenta),
                    ));

                    spans.push(Span::raw(format_default(board.title())));

                    let lines = vec![Spans::from(spans)];
                    ListItem::new(lines).style(Style::default())
                })
                .collect();

            let items = List::new(items)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(block_style.border_color().board_list()))
                        .border_type(block_style.border_type().board_list())
                        .title(format_default("Boards ")),
                )
                .highlight_style(
                    Style::default()
                        .bg(*style_prov.highlight_color())
                        .add_modifier(Modifier::BOLD),
                );

            f.render_stateful_widget(items, chunks[0], &mut app.boards.state);

            let current_board = app.selected_board().board().to_string();

            let media_url = if config.render_images && (config.image_layout == crate::config::ImageLayout::Split || config.image_layout == crate::config::ImageLayout::Hybrid) {
                match selected_field {
                    SelectedField::BoardList => None,
                    SelectedField::ThreadList => app.media_url_threads(api),
                    SelectedField::Thread => app.media_url_thread(api),
                }
            } else {
                None
            };

            let mut threads_text_rect = chunks[1];
            let mut threads_image_rect = chunks[1];
            let mut threads_image_rendered = false;
            let mut cached_split_spans = None;

            let mut image_load_failed = false;
 
            if config.render_images && (config.image_layout == crate::config::ImageLayout::Split || config.image_layout == crate::config::ImageLayout::Hybrid) && selected_field == SelectedField::ThreadList {
                if let Some(url) = &media_url {
                    threads_image_rendered = true;
                    
                    // Unified single layout splitting to avoid duplicate calculation and compiler warnings
                    let split = Layout::default()
                        .direction(Direction::Horizontal)
                        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)].as_ref())
                        .split(chunks[1]);
                    threads_text_rect = split[0];
                    threads_image_rect = split[1];
                    
                    if config.image_renderer != crate::config::ImageRenderer::Unicode {
                        active_image_url = Some(url.clone());
                        active_image_area = Some(Rect {
                            x: threads_image_rect.x + 1,
                            y: threads_image_rect.y + 1,
                            width: threads_image_rect.width.saturating_sub(2),
                            height: threads_image_rect.height.saturating_sub(2),
                        });
                    }
                    match image_cache.get_image(url, true) {
                        crate::image_cache::ImageStatus::Loaded(cached_img) => {
                            if config.image_renderer == crate::config::ImageRenderer::Unicode {
                                cached_split_spans = Some(cached_img.split.clone());
                            }
                        }
                        crate::image_cache::ImageStatus::Failed => {
                            image_load_failed = true;
                        }
                        crate::image_cache::ImageStatus::Loading => {}
                    }
                }
            }

            let selected_thread_idx = app.threads.state.selected().unwrap_or(0);
            let thread_len = app.threads.items.len();
            let threads_limit = (chunks[1].height / 6).max(3) as isize;
            let threads: Vec<ListItem> = app
                .threads
                .items
                .iter()
                .enumerate()
                .map(|(i, thread)| {
                    let is_near = (i as isize - selected_thread_idx as isize).abs() <= threads_limit;
                    let should_render_image = config.render_images && (config.image_layout == crate::config::ImageLayout::Inline || config.image_layout == crate::config::ImageLayout::Hybrid) && is_near;
                    let is_selected = config.render_images && (config.image_layout == crate::config::ImageLayout::Inline || config.image_layout == crate::config::ImageLayout::Hybrid) && i == selected_thread_idx;
                    format_post_short(
                        thread.posts().first().unwrap(),
                        i + 1,
                        thread_len,
                        threads_text_rect,
                        &image_cache,
                        api,
                        &current_board,
                        should_render_image,
                        is_selected,
                        *style_prov.highlight_color(),
                    )
                })
                .collect();

            let threads_widget = List::new(threads)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(block_style.border_color().thread_list()))
                        .border_type(block_style.border_type().thread_list())
                        .title(format_default(&format!(
                            "Threads, page {} {}",
                            thread_list.cur_page(),
                            thread_list.description(),
                        ))),
                );

            let threads_widget = if config.render_images && (config.image_layout == crate::config::ImageLayout::Inline || config.image_layout == crate::config::ImageLayout::Hybrid) {
                threads_widget
                    .highlight_style(Style::default())
                    .highlight_symbol("▶ ")
            } else {
                threads_widget
                    .highlight_style(Style::default().bg(*style_prov.highlight_color()))
            };

            f.render_stateful_widget(threads_widget, threads_text_rect, &mut app.threads.state);

            if threads_image_rendered {
                let image_widget = if config.image_renderer == crate::config::ImageRenderer::Unicode {
                    if let Some(spans) = &cached_split_spans {
                        Paragraph::new(spans.as_ref().clone())
                    } else {
                        Paragraph::new("")
                    }
                } else {
                    // Fill the inner area of the preview block with spaces to assist in clearing the old image smoothly
                    let inner_w = threads_image_rect.width.saturating_sub(2) as usize;
                    let inner_h = threads_image_rect.height.saturating_sub(2) as usize;
                    let mut lines = Vec::new();
                    
                    // Check if current terminal supports the selected image protocol
                    let is_supported = crate::graphics::is_graphics_protocol_supported(&config.image_renderer);

                    for i in 0..inner_h {
                        if !is_supported && i == 2 {
                            let protocol_name = match config.image_renderer {
                                crate::config::ImageRenderer::Iterm2 => "iTerm2",
                                crate::config::ImageRenderer::Kitty => "Kitty",
                                _ => "Selected",
                            };
                            let text = format!("  [{} protocol unsupported by terminal]", protocol_name);
                            let text_len = text.len();
                            let padding = inner_w.saturating_sub(text_len);
                            lines.push(Spans::from(format!("{}{}", text, " ".repeat(padding))));
                        } else if image_load_failed && i == 2 {
                            let text = "  [Unsupported media format]";
                            let padding = inner_w.saturating_sub(28);
                            lines.push(Spans::from(format!("{}{}", text, " ".repeat(padding))));
                        } else {
                            lines.push(Spans::from(" ".repeat(inner_w)));
                        }
                    }
                    Paragraph::new(lines)
                }
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(Color::Cyan))
                        .title(format_default(" Image Preview ")),
                );
                f.render_widget(image_widget, threads_image_rect);
            }

            let mut text_rect = chunks[2];
            let mut image_rect = chunks[2];
            let mut image_rendered = false;
            let mut cached_split_spans_post = None;
            let mut post_image_load_failed = false;
 
            if config.render_images && (config.image_layout == crate::config::ImageLayout::Split || config.image_layout == crate::config::ImageLayout::Hybrid) && selected_field == SelectedField::Thread {
                if let Some(url) = &media_url {
                    image_rendered = true;
                    
                    // Unified single layout splitting to avoid duplicate calculation and compiler warnings
                    let split = Layout::default()
                        .direction(Direction::Horizontal)
                        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)].as_ref())
                        .split(chunks[2]);
                    text_rect = split[0];
                    image_rect = split[1];
                    
                    if config.image_renderer != crate::config::ImageRenderer::Unicode {
                        active_image_url = Some(url.clone());
                        active_image_area = Some(Rect {
                            x: image_rect.x + 1,
                            y: image_rect.y + 1,
                            width: image_rect.width.saturating_sub(2),
                            height: image_rect.height.saturating_sub(2),
                        });
                    }
                    match image_cache.get_image(url, true) {
                        crate::image_cache::ImageStatus::Loaded(cached_img) => {
                            if config.image_renderer == crate::config::ImageRenderer::Unicode {
                                cached_split_spans_post = Some(cached_img.split.clone());
                            }
                        }
                        crate::image_cache::ImageStatus::Failed => {
                            post_image_load_failed = true;
                        }
                        crate::image_cache::ImageStatus::Loading => {}
                    }
                }
            }

            let selected_post_idx = app.thread.state.selected().unwrap_or(0);
            let thread_limit = (chunks[2].height / 6).max(3) as isize;
            let thread: Vec<ListItem> = app
                .thread
                .items
                .iter()
                .enumerate()
                .map(|(i, post)| {
                    let is_near = (i as isize - selected_post_idx as isize).abs() <= thread_limit;
                    let should_render_image = config.render_images && (config.image_layout == crate::config::ImageLayout::Inline || config.image_layout == crate::config::ImageLayout::Hybrid) && is_near;
                    let is_selected = config.render_images && (config.image_layout == crate::config::ImageLayout::Inline || config.image_layout == crate::config::ImageLayout::Hybrid) && i == selected_post_idx;
                    format_post_full(
                        post,
                        i + 1,
                        text_rect,
                        &image_cache,
                        api,
                        &current_board,
                        should_render_image,
                        is_selected,
                        *style_prov.highlight_color(),
                    )
                })
                .collect();

            let thread_widget = List::new(thread)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(block_style.border_color().thread()))
                        .border_type(block_style.border_type().thread())
                        .title(format_default(&format!(
                            "Thread {}",
                            app.selected_thread_description()
                        ))),
                );

            let thread_widget = if config.render_images && (config.image_layout == crate::config::ImageLayout::Inline || config.image_layout == crate::config::ImageLayout::Hybrid) {
                thread_widget
                    .highlight_style(Style::default())
                    .highlight_symbol("▶ ")
            } else {
                thread_widget
                    .highlight_style(Style::default().bg(*style_prov.highlight_color()))
            };
            f.render_stateful_widget(thread_widget, text_rect, &mut app.thread.state);

            if image_rendered {
                let image_widget = if config.image_renderer == crate::config::ImageRenderer::Unicode {
                    if let Some(spans) = &cached_split_spans_post {
                        Paragraph::new(spans.as_ref().clone())
                    } else {
                        Paragraph::new("")
                    }
                } else {
                    // Fill the inner area of the preview block with spaces to assist in clearing the old image smoothly
                    let inner_w = image_rect.width.saturating_sub(2) as usize;
                    let inner_h = image_rect.height.saturating_sub(2) as usize;
                    let mut lines = Vec::new();
                    
                    // Check if current terminal supports the selected image protocol
                    let is_supported = crate::graphics::is_graphics_protocol_supported(&config.image_renderer);

                    for i in 0..inner_h {
                        if !is_supported && i == 2 {
                            let protocol_name = match config.image_renderer {
                                crate::config::ImageRenderer::Iterm2 => "iTerm2",
                                crate::config::ImageRenderer::Kitty => "Kitty",
                                _ => "Selected",
                            };
                            let text = format!("  [{} protocol unsupported by terminal]", protocol_name);
                            let text_len = text.len();
                            let padding = inner_w.saturating_sub(text_len);
                            lines.push(Spans::from(format!("{}{}", text, " ".repeat(padding))));
                        } else if post_image_load_failed && i == 2 {
                            let text = "  [Unsupported media format]";
                            let padding = inner_w.saturating_sub(28);
                            lines.push(Spans::from(format!("{}{}", text, " ".repeat(padding))));
                        } else {
                            lines.push(Spans::from(" ".repeat(inner_w)));
                        }
                    }
                    Paragraph::new(lines)
                }
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(Color::Cyan))
                        .title(format_default(" Image Preview ")),
                );
                f.render_widget(image_widget, image_rect);
            }
            last_layout_chunk_1 = chunks[1];
            last_layout_chunk_2 = chunks[2];
        })?;

        if config.render_images && config.image_renderer != crate::config::ImageRenderer::Unicode {
            let is_supported = crate::graphics::is_graphics_protocol_supported(&config.image_renderer);

            if is_supported {
                if let Some(ref url) = active_image_url {
                    if let crate::image_cache::ImageStatus::Loaded(cached_img) = image_cache.get_image(url, true) {
                        if let Some(area) = active_image_area {
                            let max_w = area.width as f64;
                            let max_h = area.height as f64;
                            let aspect = (cached_img.width as f64 / cached_img.height as f64) * 2.0;
                            let (cols, rows) = if aspect > max_w / max_h {
                                let fit_w = max_w;
                                let fit_h = max_w / aspect;
                                (fit_w as u16, fit_h as u16)
                            } else {
                                let fit_h = max_h;
                                let fit_w = max_h * aspect;
                                (fit_w as u16, fit_h as u16)
                            };
                            let cols = cols.max(1);
                            let rows = rows.max(1);
                            let offset_x = (area.width.saturating_sub(cols) / 2) as u16;
                            let offset_y = (area.height.saturating_sub(rows) / 2) as u16;
                            let mut print_x = area.x + offset_x;
                            let print_y = area.y + offset_y;
                            
                            // iTerm2 Kitty offset correction:
                            // iTerm2's Kitty graphics protocol implementation has a 1-column left offset.
                            // Ghostty and WezTerm are perfectly centered.
                            // We check TERM_PROGRAM to apply the shift ONLY when running inside iTerm2!
                            if config.image_renderer == crate::config::ImageRenderer::Kitty {
                                let is_iterm = std::env::var("TERM_PROGRAM")
                                    .map(|val| val == "iTerm.app")
                                    .unwrap_or(false);
                                if is_iterm {
                                    print_x += 1;
                                }
                            }
                            
                            print!("\x1b[{};{}H", print_y + 1, print_x + 1);
                            if config.image_renderer == crate::config::ImageRenderer::Iterm2 {
                                print!("{}", crate::graphics::make_iterm2_sequence(&cached_img.base64_png, cols, rows));
                            } else if config.image_renderer == crate::config::ImageRenderer::Kitty {
                                print!("{}", crate::graphics::make_kitty_sequence(&cached_img.base64_png, cols, rows));
                            }
                            let _ = io::Write::flush(&mut io::stdout());
                            
                            last_image_area = Some(Rect {
                                x: print_x,
                                y: print_y,
                                width: cols,
                                height: rows,
                            });
                            last_image_url = Some(url.clone());
                        }
                    }
                }
            }
        }

        let event = events.next().unwrap();
        if let Event::Input(termion::event::Key::Char(c)) = event {
            if c.is_ascii_digit() && (has_vim_prefix || c != '0') {
                vim_prefix = vim_prefix * 10 + (c as u32 - '0' as u32);
                has_vim_prefix = true;
                continue;
            }
        }

        match event {
            Event::Input(mut input) => {
                // Normalize standard terminal CR/LF characters (which are sent by Ctrl+j/Ctrl+m)
                // so that the keybind match works successfully in standard Unix terminals.
                if let termion::event::Key::Char('\n') = input {
                    input = termion::event::Key::Ctrl('j');
                } else if let termion::event::Key::Char('\r') = input {
                    input = termion::event::Key::Ctrl('m');
                }

                let count = if has_vim_prefix {
                    let c = vim_prefix as isize;
                    vim_prefix = 0;
                    has_vim_prefix = false;
                    c
                } else {
                    1
                };

                match input {
                    _ if input == keybinds.quit => {
                        break;
                    }
                    _ if input == keybinds.left => {
                        match selected_field {
                            SelectedField::BoardList => {}
                            SelectedField::ThreadList => {
                                app.set_shown_board_list(true);
                                app.set_shown_thread(false);
                                selected_field = SelectedField::BoardList;
                            }
                            SelectedField::Thread => {
                                app.set_shown_board_list(true);
                                app.set_shown_thread_list(true);
                                app.set_shown_thread(false);
                                selected_field = SelectedField::ThreadList;
                            }
                        };
                    }
                    _ if input == keybinds.down => {
                        app.advance(&selected_field, count);
                    }
                    _ if input == keybinds.up => {
                        app.advance(&selected_field, -1 * count);
                    }
                    _ if input == keybinds.quick_down => {
                        app.advance(&selected_field, 5 * count);
                    }
                    _ if input == keybinds.quick_up => {
                        app.advance(&selected_field, -5 * count);
                    }
                _ if input == keybinds.fullscreen => {
                    match selected_field {
                        SelectedField::BoardList => {
                            if app.shown_thread_list() {
                                app.toggle_shown_board_list();
                                selected_field = SelectedField::ThreadList;
                            }
                        }
                        SelectedField::ThreadList => {
                            if app.shown_thread() {
                                app.toggle_shown_thread_list();
                                selected_field = SelectedField::Thread;
                            } else {
                                app.toggle_shown_board_list();
                                selected_field = SelectedField::ThreadList;
                            }
                        }
                        SelectedField::Thread => {
                            app.toggle_shown_thread_list();
                            selected_field = SelectedField::Thread;
                        }
                    };
                }
                _ if input == keybinds.help => {
                    app.help_bar_mut().toggle_shown();
                }
                _ if input == keybinds.open_thread => {
                    let url = match selected_field {
                        SelectedField::BoardList => app.url_boards(api),
                        SelectedField::ThreadList => app.url_threads(api),
                        SelectedField::Thread => app.url_thread(api),
                    };

                    open_in_browser(url).expect("Browser error.");
                }
                _ if input == keybinds.open_media => {
                    let url = match selected_field {
                        SelectedField::BoardList => None,
                        SelectedField::ThreadList => app.media_url_threads(api),
                        SelectedField::Thread => app.media_url_thread(api),
                    };

                    if let Some(url) = url {
                        open_in_browser(url).expect("Browser error.");
                    }
                }
                _ if input == keybinds.copy_thread => {
                    let url = match selected_field {
                        SelectedField::BoardList => app.url_boards(api),
                        SelectedField::ThreadList => app.url_threads(api),
                        SelectedField::Thread => app.url_thread(api),
                    };

                    ctx.set_contents(url).expect("Clipboard error.");
                }
                _ if input == keybinds.copy_media => {
                    let url = match selected_field {
                        SelectedField::BoardList => None,
                        SelectedField::ThreadList => app.media_url_threads(api),
                        SelectedField::Thread => app.media_url_thread(api),
                    };

                    if let Some(url) = url {
                        ctx.set_contents(url).expect("Clipboard error.");
                    }
                }
                _ if input == keybinds.page_next => {
                    match selected_field {
                        SelectedField::ThreadList => {
                            let mut threads: Vec<Thread> = vec![];
                            runtime.block_on(async {
                                let result = client
                                    .get_threads(
                                        app.selected_board().board(),
                                        thread_list.next_page(app.selected_board()),
                                    )
                                    .await;
                                match result {
                                    Ok(data) => threads = data,
                                    Err(err) => eprintln!("{:#?}", err),
                                };

                                app.fill_threads(threads);
                            });
                        }
                        _ => {}
                    };
                }
                _ if input == keybinds.page_previous => {
                    match selected_field {
                        SelectedField::ThreadList => {
                            let mut threads: Vec<Thread> = vec![];
                            runtime.block_on(async {
                                let result = client
                                    .get_threads(
                                        app.selected_board().board(),
                                        thread_list.prev_page(app.selected_board()),
                                    )
                                    .await;
                                match result {
                                    Ok(data) => threads = data,
                                    Err(err) => eprintln!("{:#?}", err),
                                };

                                app.fill_threads(threads);
                            });
                        }
                        _ => {}
                    };
                }
                _ if input == keybinds.reload => {
                    match selected_field {
                        SelectedField::ThreadList => {
                            let mut threads: Vec<Thread> = vec![];
                            runtime.block_on(async {
                                let result = client
                                    .get_threads(
                                        app.selected_board().board(),
                                        thread_list.cur_page(),
                                    )
                                    .await;
                                match result {
                                    Ok(data) => threads = data,
                                    Err(err) => eprintln!("{:#?}", err),
                                };

                                app.fill_threads(threads);
                                app.threads.advance_by(1);
                            });
                        }
                        SelectedField::Thread => {
                            let mut thread: Vec<ThreadPost> = vec![];
                            runtime.block_on(async {
                                let result = client
                                    .get_thread(
                                        app.selected_board().board(),
                                        app.selected_thread().posts().first().unwrap().no() as u64,
                                    )
                                    .await;
                                match result {
                                    Ok(data) => thread = data,
                                    Err(err) => eprintln!("{:#?}", err),
                                };

                                app.fill_thread(thread);
                                app.thread.advance_by(1);
                            });
                        }
                        _ => {}
                    };
                }
                _ if input == keybinds.right => {
                    match selected_field {
                        SelectedField::BoardList => {
                            selected_field = SelectedField::ThreadList;
                            app.set_shown_thread_list(true);

                            thread_list = ThreadList::new();
                            thread_list.set_description(app.selected_board().meta_description());
                            let mut threads: Vec<Thread> = vec![];
                            runtime.block_on(async {
                                let result = client
                                    .get_threads(
                                        app.selected_board().board(),
                                        thread_list.cur_page(),
                                    )
                                    .await;
                                match result {
                                    Ok(data) => threads = data,
                                    Err(err) => eprintln!("{:#?}", err),
                                };

                                app.fill_threads(threads);
                                app.threads.advance_by(1);
                            });
                        }
                        SelectedField::ThreadList => {
                            selected_field = SelectedField::Thread;
                            app.set_shown_thread(true);
                            app.set_shown_board_list(false);

                            let mut thread: Vec<ThreadPost> = vec![];
                            runtime.block_on(async {
                                let result = client
                                    .get_thread(
                                        app.selected_board().board(),
                                        app.selected_thread().posts().first().unwrap().no() as u64,
                                    )
                                    .await;
                                match result {
                                    Ok(data) => thread = data,
                                    Err(err) => eprintln!("{:#?}", err),
                                };

                                app.fill_thread(thread);
                                app.thread.advance_by(1);
                            });
                        }
                        _ => {}
                    };
                }
                _ if input == keybinds.toggle_image_previews => {
                    if config.render_images {
                        let is_supported = crate::graphics::is_graphics_protocol_supported(&config.image_renderer);

                        if config.image_renderer == crate::config::ImageRenderer::Kitty && is_supported {
                            print!("{}", crate::graphics::make_kitty_clear_sequence());
                            let _ = io::Write::flush(&mut io::stdout());
                        }

                        if config.image_layout == crate::config::ImageLayout::Split || config.image_layout == crate::config::ImageLayout::Hybrid {
                            let target_chunk = match selected_field {
                                SelectedField::Thread => last_layout_chunk_2,
                                _ => last_layout_chunk_1,
                            };
                            let split = Layout::default()
                                .direction(Direction::Horizontal)
                                .constraints([Constraint::Percentage(60), Constraint::Percentage(40)].as_ref())
                                .split(target_chunk);
                            let prev_box = split[1];
                            for r in 0..prev_box.height {
                                print!("\x1b[{};{}H{}", prev_box.y + r + 1, prev_box.x + 1, " ".repeat(prev_box.width as usize));
                            }
                            let _ = io::Write::flush(&mut io::stdout());
                        }

                        if let Some(area) = last_image_area {
                            if config.image_renderer == crate::config::ImageRenderer::Iterm2 && is_supported {
                                for r in 0..area.height {
                                    print!("\x1b[{};{}H{}", area.y + r + 1, area.x + 1, " ".repeat(area.width as usize));
                                }
                                let _ = io::Write::flush(&mut io::stdout());
                            }
                        }
                    }

                    config.render_images = !config.render_images;
                    last_image_area = None;
                    last_image_url = None;
                }
                _ => {}
            }
            },
            Event::Tick => {
                app.advance_idly();
            }
        }
    }

    if config.render_images && config.image_renderer == crate::config::ImageRenderer::Kitty {
        print!("{}", crate::graphics::make_kitty_clear_sequence());
        let _ = io::Write::flush(&mut io::stdout());
    }

    Ok(())
}
