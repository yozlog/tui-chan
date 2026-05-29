use std::collections::HashSet;
use tui::backend::Backend;
use tui::layout::{Constraint, Direction, Layout, Rect};
use tui::style::{Color, Modifier, Style};
use tui::text::{Span, Spans};
use tui::widgets::{Block, Borders, List, ListItem, Paragraph, Wrap};
use tui::Frame;

use crate::app::App;
use crate::config::Config;
use crate::format::{format_post_full, format_post_short};
use crate::image_cache::ImageCache;
use crate::model::ThreadList;
use crate::style::{SelectedField, StyleProvider};

fn build_image_preview_placeholder<'a>(
    config: &Config,
    rect: Rect,
    load_failed: bool,
) -> Paragraph<'a> {
    let inner_w = rect.width.saturating_sub(2) as usize;
    let inner_h = rect.height.saturating_sub(2) as usize;
    let mut lines = Vec::new();
    
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
        } else if load_failed && i == 2 {
            let text = "  [Unsupported media format]";
            let padding = inner_w.saturating_sub(28);
            lines.push(Spans::from(format!("{}{}", text, " ".repeat(padding))));
        } else {
            lines.push(Spans::from(" ".repeat(inner_w)));
        }
    }
    Paragraph::new(lines)
}

/// helper function to create a centered rect using up certain percentage of the available rect `r`
fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints(
            [
                Constraint::Percentage((100 - percent_y) / 2),
                Constraint::Percentage(percent_y),
                Constraint::Percentage((100 - percent_y) / 2),
            ]
            .as_ref(),
        )
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints(
            [
                Constraint::Percentage((100 - percent_x) / 2),
                Constraint::Percentage(percent_x),
                Constraint::Percentage((100 - percent_x) / 2),
            ]
            .as_ref(),
        )
        .split(popup_layout[1])[1]
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn draw<B: Backend>(
    f: &mut Frame<B>,
    app: &mut App,
    config: &Config,
    style_prov: &StyleProvider,
    selected_field: &SelectedField,
    image_cache: &ImageCache,
    api: &dyn crate::client::api::ContentUrlProvider,
    thread_list: &ThreadList,
    active_image_url: &mut Option<String>,
    active_image_area: &mut Option<Rect>,
    last_layout_chunk_1: &mut Rect,
    last_layout_chunk_2: &mut Rect,
) {
            let block_style = style_prov.default_from_selected_field(selected_field);
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
                            format!(" /{}/", board.board())
                        },
                        Style::default().fg(Color::Magenta),
                    ));

                    spans.push(Span::raw(format!(" {}", board.title())));

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
                        .title(" Boards "),
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
 
            if config.render_images && (config.image_layout == crate::config::ImageLayout::Split || config.image_layout == crate::config::ImageLayout::Hybrid) && *selected_field == SelectedField::ThreadList {
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
                        *active_image_url = Some(url.clone());
                        *active_image_area = Some(Rect {
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
                        image_cache,
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
                        .title(format!(
                            " Threads, page {} {} ",
                            thread_list.cur_page(),
                            thread_list.description(),
                        )),
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
                    build_image_preview_placeholder(config, threads_image_rect, image_load_failed)
                }
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(Color::Cyan))
                        .title(" Image Preview "),
                );
                f.render_widget(image_widget, threads_image_rect);
            }


            let mut text_rect = chunks[2];
            let mut image_rect = chunks[2];
            let mut image_rendered = false;
            let mut cached_split_spans_post = None;
            let mut post_image_load_failed = false;
 
            if config.render_images && (config.image_layout == crate::config::ImageLayout::Split || config.image_layout == crate::config::ImageLayout::Hybrid) && *selected_field == SelectedField::Thread {
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
                        *active_image_url = Some(url.clone());
                        *active_image_area = Some(Rect {
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
                        image_cache,
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
                        .title(format!(
                            " Thread {} ",
                            app.selected_thread_description()
                        )),
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
                    build_image_preview_placeholder(config, image_rect, post_image_load_failed)
                }
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(Color::Cyan))
                        .title(" Image Preview "),
                );
                f.render_widget(image_widget, image_rect);
            }
            if app.native_board_search.active {
                // Clear the background to draw a centered popup floating over the UI
                let area = centered_rect(50, 50, f.size());
                f.render_widget(tui::widgets::Clear, area);

                let search_block = Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Blue));

                let inner_area = search_block.inner(area);
                f.render_widget(search_block, area);

                // Split inner area into input (top line) and list (rest)
                let chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Length(1), Constraint::Min(0)].as_ref())
                    .split(inner_area);

                let top_chunks = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([Constraint::Min(0), Constraint::Length(10)].as_ref())
                    .split(chunks[0]);

                // Render Input
                let input_spans = vec![
                    Span::styled("Search Board> ", Style::default().fg(Color::Cyan)),
                    Span::styled(&app.native_board_search.query, Style::default().fg(Color::White)),
                    Span::styled(" ", Style::default().bg(Color::Gray)), // Cursor
                ];
                let input_paragraph = Paragraph::new(Spans::from(input_spans));
                f.render_widget(input_paragraph, top_chunks[0]);
                
                // Info count
                let matches_len = app.native_board_search.matched_indices.len();
                let total_len = app.boards.items.len();
                let info_text = format!("{}/{}", matches_len, total_len);
                let info_paragraph = Paragraph::new(info_text).alignment(tui::layout::Alignment::Right).style(Style::default().fg(Color::Yellow));
                // We render it on the same line as input, aligned right
                f.render_widget(info_paragraph, top_chunks[1]);

                // Render Matches
                let start_idx = app.native_board_search.selected.saturating_sub((chunks[1].height as usize) / 2);
                let end_idx = (start_idx + chunks[1].height as usize).min(matches_len);
                
                let visible_matches = app.native_board_search.matched_indices.iter().enumerate().skip(start_idx).take(end_idx - start_idx);
                
                let mut list_items = Vec::new();
                for (i, (board_idx, char_indices)) in visible_matches {
                    let board = &app.boards.items[*board_idx];
                    let text = format!("/{}/ {}", board.board(), board.title());
                    
                    let mut text_spans = Vec::new();
                    let is_selected = i == app.native_board_search.selected;
                    let bg_color = if is_selected { Color::DarkGray } else { Color::Reset };
                    
                    if is_selected {
                        text_spans.push(Span::styled("> ", Style::default().fg(Color::Magenta).bg(bg_color)));
                    } else {
                        text_spans.push(Span::styled("  ", Style::default().bg(bg_color)));
                    }
                    
                    let highlight_set: HashSet<usize> = char_indices.iter().copied().collect();
                    for (c_idx, c) in text.chars().enumerate() {
                        let mut style = Style::default().bg(bg_color);
                        if highlight_set.contains(&c_idx) {
                            style = style.fg(Color::Magenta);
                        }
                        text_spans.push(Span::styled(c.to_string(), style));
                    }
                    
                    // Pad with spaces for the background bar to extend
                    if is_selected {
                        let padding = chunks[1].width.saturating_sub(text.chars().count() as u16 + 2);
                        text_spans.push(Span::styled(" ".repeat(padding as usize), Style::default().bg(bg_color)));
                    }

                    list_items.push(ListItem::new(Spans::from(text_spans)));
                }

                let list = List::new(list_items);
                f.render_widget(list, chunks[1]);
            }

            *last_layout_chunk_1 = chunks[1];
            *last_layout_chunk_2 = chunks[2];
}
