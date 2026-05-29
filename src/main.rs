#![allow(clippy::single_match)]

use std::{
    env, str, thread,
    io::{self, Write},
    process::{self, Command, Stdio},
    time::Duration,
};

use fuzzy_matcher::FuzzyMatcher;
use fuzzy_matcher::skim::SkimMatcherV2;

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
mod ui;

use crate::config::read_or_create_config_file;
use crate::image_cache::ImageCache;

fn update_native_search_matches(app: &mut app::App) {
    let query = app.native_board_search.query.clone();
    let matcher = SkimMatcherV2::default();
    
    let mut matches = Vec::new();
    for (i, board) in app.boards.items.iter().enumerate() {
        let b = board.board();
        let stripped_text = format!("{} {}", b, board.title());
        if query.is_empty() {
            matches.push((i, vec![], 0));
        } else if let Some((score, indices)) = matcher.fuzzy_indices(&stripped_text, &query) {
            let b_len = b.chars().count();
            // Shift indices to account for the leading '/' in the display format "/board/ title"
            let shifted_indices = indices.into_iter().map(|idx| {
                if idx < b_len {
                    idx + 1  // inside the board name: account for leading '/'
                } else {
                    idx + 2  // inside the title: account for leading '/' and trailing '/'
                }
            }).collect();
            matches.push((i, shifted_indices, score));
        }
    }
    
    if !query.is_empty() {
        matches.sort_by_key(|b| std::cmp::Reverse(b.2));
    }
    
    app.native_board_search.matched_indices = matches.into_iter().map(|(i, ind, _)| (i, ind)).collect();
    app.native_board_search.selected = 0;
}

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
                        update_native_search_matches(&mut app);
                    }
                    Key::Char(c) if !c.is_control() => {
                        app.native_board_search.query.push(c);
                        update_native_search_matches(&mut app);
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
                _ if input == keybinds.search_board
                    && config.fzf_board_search && selected_field == SelectedField::BoardList => {
                        if config.board_search_backend == crate::config::BoardSearchBackend::External {
                            let boards_str = app.boards_mut().items()
                                .iter()
                                .map(|b| {
                                    let board = b.board();
                                    format!("{} {}\t/{}/ {}", board, b.title(), board, b.title())
                                })
                                .collect::<Vec<String>>()
                                .join("\n");
                                
                            // Suspend terminal
                            let _ = crossterm::terminal::disable_raw_mode();
                            let _ = crossterm::execute!(io::stdout(), crossterm::event::DisableMouseCapture);
                            events.pause();
                            
                            // Wait slightly longer than the event thread's poll timeout (50ms) 
                            // to ensure it fully enters the paused state and stops reading stdin.
                            // Otherwise, it might intercept fzf's cursor position query (\x1b[6n), 
                            // causing fzf to timeout and delay startup by exactly 1 second!
                            thread::sleep(Duration::from_millis(60));

                            let child_res = Command::new("fzf")
                                .args(["--height", "50%", "--layout=reverse", "--border=rounded", "--prompt=Search Board> "])
                                .args(["--delimiter=\t", "--with-nth=2", "--nth=1"])
                                .arg("--color=bg+:-1,hl:magenta,hl+:magenta,prompt:cyan,pointer:magenta,border:blue,info:yellow")
                                .stdin(Stdio::piped())
                                .stdout(Stdio::piped())
                                .spawn();
                                
                            match child_res {
                                Ok(mut child) => {
                                    if let Some(mut stdin) = child.stdin.take() {
                                        let _ = stdin.write_all(boards_str.as_bytes());
                                    }
                                    
                                    if let Ok(output) = child.wait_with_output() {
                                        if output.status.success() {
                                            let result = String::from_utf8_lossy(&output.stdout);
                                            // result is "<board> <title>\t/board/ <title>\n"; extract board name
                                            let selected_board = result.split('\t').next().unwrap_or("").split(' ').next().unwrap_or("").trim();
                                            if let Some(index) = app.boards_mut().items().iter().position(|b| b.board() == selected_board) {
                                                app.boards_mut().select_index(index);
                                            }
                                        }
                                    }
                                }
                                Err(e) => {
                                    eprintln!("[tui-chan] Failed to launch fzf: {}. Is it installed?", e);
                                }
                            }
                            
                            // Restore terminal
                            events.resume();
                            let _ = crossterm::execute!(io::stdout(), crossterm::event::EnableMouseCapture);
                            let _ = crossterm::terminal::enable_raw_mode();
                            let _ = terminal.clear();
                        } else {
                            // Native search
                            app.native_board_search.active = true;
                            app.native_board_search.query.clear();
                            update_native_search_matches(&mut app);
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

    disable_raw_mode()?;
    execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture)?;

    Ok(())
}
