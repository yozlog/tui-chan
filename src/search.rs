use std::{
    io::{self, Write},
    process::{Command, Stdio},
    thread,
    time::Duration,
};

use fuzzy_matcher::skim::SkimMatcherV2;
use fuzzy_matcher::FuzzyMatcher;

use crate::app::App;
use crate::event::Events;

pub fn update_native_search_matches(app: &mut App) {
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

pub fn run_external_fzf<B: tui::backend::Backend>(
    app: &mut App,
    events: &Events,
    terminal: &mut tui::Terminal<B>,
) {
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
}
