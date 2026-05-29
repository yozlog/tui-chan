#![allow(clippy::single_match)]

use std::{
    env, str,
    io,
    process,
};


use client::ChanClient;
use clipboard::{ClipboardContext, ClipboardProvider};
use open::that as open_in_browser;

use crossterm::{
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    event::{EnableMouseCapture, DisableMouseCapture},
};
use tokio::runtime::Runtime;
use tui::backend::CrosstermBackend;
use tui::layout::{Constraint, Direction, Layout};
use tui::Terminal;

use crate::app::App;
use crate::client::api::{
    from_name as channel_provider_from_name, ChannelProvider, ContentUrlProvider,
};
use crate::event::{Event, Events, Key};
use crate::keybinds::{read_or_create_keybinds_file, Keybinds};
use crate::model::{Board, ThreadList};
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
mod ui;
mod search;

macro_rules! fetch_threads {
    ($runtime:expr, $client:expr, $board:expr, $page:expr, $app:expr, $on_success:expr) => {
        $runtime.block_on(async {
            match $client.get_threads($board, $page).await {
                Ok(data) => {
                    if !data.is_empty() {
                        $app.fill_threads(data);
                        $on_success
                    }
                }
                Err(err) => eprintln!("{:#?}", err),
            }
        });
    };
}

macro_rules! fetch_thread {
    ($runtime:expr, $client:expr, $board:expr, $no:expr, $app:expr, $on_success:expr) => {
        $runtime.block_on(async {
            match $client.get_thread($board, $no).await {
                Ok(data) => {
                    if !data.is_empty() {
                        $app.fill_thread(data);
                        $on_success
                    }
                }
                Err(err) => eprintln!("{:#?}", err),
            }
        });
    };
}

use crate::config::read_or_create_config_file;
use crate::image_cache::ImageCache;


fn main() -> Result<(), io::Error> {
    // Get keybinds from config file
    let keybinds = read_or_create_keybinds_file().expect("Failed to read keybinds file");
    let keybinds = Keybinds::parse_from_file(&keybinds).expect("Failed to parse keybinds file");

    // Load settings from settings.conf file
    let mut config = read_or_create_config_file().expect("Failed to read config file");


    let args: Vec<String> = env::args().collect();
    let chan: &str = if args.len() == 1 { "default" } else { &args[1] };

    let api_provider: &dyn ChannelProvider = match channel_provider_from_name(chan) {
        Some(api) => api,
        None => {
            println!("Imageboard name \"{}\" is not valid.", chan);
            process::exit(1);
        }
    };

    let reqwest_client = reqwest::Client::builder()
        .user_agent(concat!(
            env!("CARGO_PKG_NAME"),
            "/",
            env!("CARGO_PKG_VERSION")
        ))
        .build()
        .unwrap();

    let client = ChanClient::new(reqwest_client.clone(), api_provider.as_api());
    let api: &dyn ContentUrlProvider = api_provider.as_content();

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    let runtime = Runtime::new()?;
    let tokio_handle = runtime.handle().clone();
    let events = Events::new();
    let image_cache = ImageCache::new(tokio_handle, events.tx(), reqwest_client);

    let mut boards: Vec<Board> = vec![];
    runtime.block_on(async {
        let result = client.get_boards().await;

        match result {
            Ok(data) => boards = data,
            Err(e) => {
                let _ = disable_raw_mode();
                let _ = execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture);
                eprintln!("Could not fetch boards: {:?}", e);
                process::exit(1);
            }
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
    let mut has_top_prefix = false;

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
            crate::ui::draw(
                f,
                &mut app,
                &config,
                &style_prov,
                &selected_field,
                &image_cache,
                api,
                &thread_list,
                &mut active_image_url,
                &mut active_image_area,
                &mut last_layout_chunk_1,
                &mut last_layout_chunk_2,
            );
        })?;

        if config.render_images && config.image_renderer != crate::config::ImageRenderer::Unicode {
            let is_supported = crate::graphics::is_graphics_protocol_supported(&config.image_renderer);

            if is_supported {
                let current_url = active_image_url.clone();
                let needs_redraw = last_image_url != current_url || last_image_area != active_image_area;

                if needs_redraw {
                    let mut drew = false;

                    if let Some(ref url) = current_url {
                        match image_cache.get_image(url, true) {
                            crate::image_cache::ImageStatus::Loaded(cached_img) => {
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
                                    let offset_x = area.width.saturating_sub(cols) / 2;
                                    let offset_y = area.height.saturating_sub(rows) / 2;
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
                                    
                                    last_image_area = active_image_area;
                                    last_image_url = current_url.clone();
                                    drew = true;
                                }
                            },
                            crate::image_cache::ImageStatus::Failed => {
                                last_image_url = current_url.clone();
                                last_image_area = active_image_area;
                                drew = true;
                            },
                            crate::image_cache::ImageStatus::Loading => {
                                // Do nothing, let it check again next tick
                            }
                        }
                    }

                    if !drew && current_url.is_none() {
                        last_image_url = None;
                        last_image_area = None;
                    }
                }
            }
        }

        let event = events.next().unwrap();
        if app.native_board_search.active {
            if let Event::Input(input) = event {
                match input {
                    Key::Esc | Key::Ctrl('c') => {
                        app.native_board_search.active = false;
                    }
                    Key::Char('\n') | Key::Ctrl('j') | Key::Ctrl('m') => {
                        app.native_board_search.active = false;
                        let selected_board_idx = app.native_board_search.matched_indices.get(app.native_board_search.selected).map(|(idx, _)| *idx);
                        if let Some(idx) = selected_board_idx {
                            app.boards_mut().select_index(idx);
                        }
                    }
                    Key::Up | Key::Ctrl('p') => {
                        app.native_board_search.selected = app.native_board_search.selected.saturating_sub(1);
                    }
                    Key::Down | Key::Ctrl('n')
                        if !app.native_board_search.matched_indices.is_empty() => {
                        app.native_board_search.selected = (app.native_board_search.selected + 1)
                            .min(app.native_board_search.matched_indices.len() - 1);
                    }
                    Key::Backspace => {
                        app.native_board_search.query.pop();
                        search::update_native_search_matches(&mut app);
                    }
                    Key::Char(c) if !c.is_control() => {
                        app.native_board_search.query.push(c);
                        search::update_native_search_matches(&mut app);
                    }
                    _ => {}
                }
            }
            continue;
        }

        if let Event::Input(Key::Char(c)) = event {
            if c.is_ascii_digit() && (has_vim_prefix || c != '0') {
                vim_prefix = vim_prefix * 10 + (c as u32 - '0' as u32);
                has_vim_prefix = true;
                continue;
            }
        }

        if let Event::Input(input) = event {
            if input == keybinds.top && !has_top_prefix {
                has_top_prefix = true;
                continue;
            }
            has_top_prefix = false;
        }

        match event {
            Event::Input(input) => {

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
                        app.advance(&selected_field, -count);
                    }
                    _ if input == keybinds.quick_down => {
                        app.advance(&selected_field, 5 * count);
                    }
                    _ if input == keybinds.quick_up => {
                        app.advance(&selected_field, -5 * count);
                    }
                    _ if input == keybinds.top => {
                        app.jump_top(&selected_field);
                    }
                    _ if input == keybinds.bottom => {
                        app.jump_bottom(&selected_field);
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
                    open_in_browser(app.current_url(&selected_field, api)).expect("Browser error.");
                }
                _ if input == keybinds.open_media => {
                    if let Some(url) = app.current_media_url(&selected_field, api) {
                        open_in_browser(url).expect("Browser error.");
                    }
                }
                _ if input == keybinds.copy_thread => {
                    ctx.set_contents(app.current_url(&selected_field, api)).expect("Clipboard error.");
                }
                _ if input == keybinds.copy_media => {
                    if let Some(url) = app.current_media_url(&selected_field, api) {
                        ctx.set_contents(url).expect("Clipboard error.");
                    }
                }
                _ if input == keybinds.search_board
                    && config.fzf_board_search && selected_field == SelectedField::BoardList => {
                        if config.board_search_backend == crate::config::BoardSearchBackend::External {
                            search::run_external_fzf(&mut app, &events, &mut terminal);
                        } else {
                            // Native search
                            app.native_board_search.active = true;
                            app.native_board_search.query.clear();
                            search::update_native_search_matches(&mut app);
                        }
                }
                _ if input == keybinds.page_next => {
                    match selected_field {
                        SelectedField::ThreadList => {
                            fetch_threads!(runtime, client, app.selected_board().board(), thread_list.next_page(app.selected_board()), app, {});
                        }
                        _ => {}
                    };
                }
                _ if input == keybinds.page_previous => {
                    match selected_field {
                        SelectedField::ThreadList => {
                            fetch_threads!(runtime, client, app.selected_board().board(), thread_list.prev_page(app.selected_board()), app, {});
                        }
                        _ => {}
                    };
                }
                _ if input == keybinds.reload => {
                    match selected_field {
                        SelectedField::ThreadList => {
                            fetch_threads!(runtime, client, app.selected_board().board(), thread_list.cur_page(), app, { app.threads.advance_by(1); });
                        }
                        SelectedField::Thread => {
                            fetch_thread!(runtime, client, app.selected_board().board(), app.selected_thread().posts().first().unwrap().no() as u64, app, { app.thread.advance_by(1); });
                        }
                        _ => {}
                    };
                }
                _ if input == keybinds.right => {
                    match selected_field {
                        SelectedField::BoardList => {
                            thread_list = ThreadList::new();
                            thread_list.set_description(app.selected_board().meta_description());
                            fetch_threads!(runtime, client, app.selected_board().board(), thread_list.cur_page(), app, {
                                selected_field = SelectedField::ThreadList;
                                app.set_shown_thread_list(true);
                                app.threads.advance_by(1);
                            });
                        }
                        SelectedField::ThreadList => {
                            fetch_thread!(runtime, client, app.selected_board().board(), app.selected_thread().posts().first().unwrap().no() as u64, app, {
                                selected_field = SelectedField::Thread;
                                app.set_shown_thread(true);
                                app.set_shown_board_list(false);
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

    disable_raw_mode()?;
    execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture)?;

    Ok(())
}
