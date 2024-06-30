use std::{
    collections::{HashMap, HashSet},
    fs::File,
    io::{self, Read},
    rc::Rc,
};

use anyhow::{bail, Context};
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use indicatif::HumanBytes;
use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span},
    widgets::{
        Block, Borders, Cell, Clear, Padding, Paragraph, Row, StatefulWidget, Table, TableState,
        Widget, Wrap,
    },
    Frame,
};

use crate::{
    config::repository::{RemotePackage, RepositoryXml},
    get_home,
    submodules::{
        sdk::InstalledPackage,
        sdkmanager::filters::{FilteredPackages, SdkFilters},
    },
};

use super::Tui;

/// List of pages we can switch between
#[derive(Default)]
enum Pages {
    #[default]
    MainList,
    License,
    // Installed,
}

#[derive(Default)]
enum Modes {
    #[default]
    Normal,
    FilterInput,
}

/// A help entry to be shown on help popup
struct HelpEntry {
    key: String,
    help: String,
}

impl HelpEntry {
    pub fn new(key: &str, help: &str) -> Self {
        Self {
            key: key.to_string(),
            help: help.to_string(),
        }
    }
}

#[derive(Default)]
struct HelpFooter {}

impl StatefulWidget for HelpFooter {
    type State = Vec<HelpEntry>;

    fn render(
        self,
        area: ratatui::prelude::Rect,
        buf: &mut ratatui::prelude::Buffer,
        state: &mut Self::State,
    ) {
        let mut spans: Vec<Span> = Vec::new();
        for h in state {
            spans.push(Span::styled(
                format!(" {} ", h.key),
                Style::new().fg(Color::DarkGray),
            ));
            spans.push(Span::from(h.help.as_str()));
        }
        let paragraph = Paragraph::new(Line::from(spans)).wrap(Wrap { trim: false });
        paragraph.render(area, buf);
    }
}
#[derive(Default)]
struct HelpPopoup {
    percent_width: u16,
    percent_height: u16,
    scroll_position: u16,
    help: HashMap<String, Vec<HelpEntry>>,
}

impl HelpPopoup {
    pub fn new(percent_width: u16, percent_height: u16) -> Self {
        Self {
            percent_width,
            percent_height,
            scroll_position: 0,
            help: HashMap::new(),
        }
    }
    fn calculate_area(&self, area: Rect) -> Rect {
        // Calculate the position of the popup based on width&height
        let center_vertical = Layout::vertical([
            Constraint::Percentage((100 - self.percent_height) / 2),
            Constraint::Percentage(self.percent_height),
            Constraint::Percentage((100 - self.percent_height) / 2),
        ])
        .split(area);
        let center_horizontal = Layout::horizontal([
            Constraint::Percentage((100 - self.percent_width) / 2),
            Constraint::Percentage(self.percent_width),
            Constraint::Percentage((100 - self.percent_width) / 3),
        ])
        .split(center_vertical[1]);

        center_horizontal[1]
    }
    pub fn set_help(&mut self, context: String, entries: Vec<HelpEntry>) {
        self.help.insert(context, entries);
    }
    pub fn draw(&mut self, frame: &mut Frame) {
        let area = self.calculate_area(frame.size());
        frame.render_widget(Clear, area);
        frame.render_widget(Block::new().title("Help").borders(Borders::ALL), area);
        frame.render_widget(
            self,
            Rect::new(area.x + 1, area.y + 1, area.width - 1, area.height - 1),
        );
    }
}
impl Widget for &mut HelpPopoup {
    fn render(self, area: Rect, buf: &mut ratatui::prelude::Buffer)
    where
        Self: Sized,
    {
        let layout = Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]).split(area);
        Paragraph::new("LABt sdkmanager")
            .alignment(ratatui::layout::Alignment::Center)
            .render(layout[0], buf);

        let mut keys: Vec<&String> = self.help.keys().collect();
        // sort since this is a hashmap, order cannot be guaranteed
        keys.sort_unstable();

        let mut lines: Vec<Line> = Vec::new();
        for key in keys {
            if let Some(entries) = self.help.get(key) {
                // Title for section
                lines.push(Line::raw(""));
                lines.push(Line::styled(key, Style::new().bold().underlined()));
                lines.extend(entries.iter().map(|help| {
                    Line::from(vec![
                        Span::styled(
                            format!("{}  ", help.key.as_str()),
                            Style::new().fg(Color::DarkGray),
                        ),
                        Span::from(help.help.as_str()),
                    ])
                }));
            }
        }

        Paragraph::new(lines)
            .scroll((self.scroll_position, 0))
            .block(Block::new().padding(Padding::proportional(1)))
            .wrap(Wrap { trim: false })
            .render(layout[1], buf);
    }
}

#[derive(Default)]
struct MainListPage {}

impl StatefulWidget for &MainListPage {
    type State = AppState;
    fn render(
        self,
        area: ratatui::prelude::Rect,
        buf: &mut ratatui::prelude::Buffer,
        state: &mut Self::State,
    ) {
        let layout = Layout::new(
            ratatui::layout::Direction::Vertical,
            [
                Constraint::Length(1),
                Constraint::Percentage(50),
                Constraint::Fill(1),
            ],
        )
        .split(area);
        // page title
        Paragraph::new("Available packages").render(layout[0], buf);

        let header_style = Style::new().fg(Color::DarkGray);
        let header = ["Name", "Version", "Path"]
            .into_iter()
            .map(Cell::from)
            .collect::<Row>()
            .style(header_style)
            .height(1);
        let mut longest_version_string = 7; // default value equal to "version".len()

        let selected_style = Style::default()
            .add_modifier(Modifier::REVERSED)
            .bg(Color::Black);

        let rows: Vec<Row> = state
            .get_remote_packages()
            .iter()
            .map(|package| {
                let name_cell = Cell::new(package.get_display_name().as_str())
                    .style(Style::new().fg(Color::Blue));

                let revision = package.get_revision();
                let version_string = format!(
                    "{}.{}.{}.{}",
                    revision.major, revision.minor, revision.micro, revision.preview
                );
                if version_string.len() > longest_version_string {
                    longest_version_string = version_string.len();
                }
                let version_cell = Cell::new(version_string);
                let path = Cell::new(package.get_path().as_str());

                Row::new(vec![name_cell, version_cell, path])
            })
            .collect();

        let table = Table::new(
            rows,
            [
                Constraint::Fill(2),
                Constraint::Length(longest_version_string as u16),
                Constraint::Fill(2),
            ],
        )
        .header(header)
        .highlight_style(selected_style)
        .column_spacing(1);
        ratatui::widgets::StatefulWidget::render(
            table,
            layout[1],
            buf,
            &mut state.table_state.clone(),
        );

        let details = DetailsWidget::default();
        let inner = layout[2].inner(&ratatui::layout::Margin {
            horizontal: 1,
            vertical: 1,
        });
        let block = Block::new().borders(Borders::TOP);
        block.render(layout[2], buf);
        StatefulWidget::render(&details, inner, buf, state);
    }
}

#[derive(Default)]
struct LicensePage {}

impl StatefulWidget for &LicensePage {
    type State = AppState;
    fn render(
        self,
        area: ratatui::prelude::Rect,
        buf: &mut ratatui::prelude::Buffer,
        state: &mut Self::State,
    ) {
        let scroll = state.license_scroll_position as u16;
        let block = Block::new()
            .padding(Padding::symmetric(2, 2))
            .borders(Borders::ALL);

        match state.get_selected_license() {
            Ok(Some((id, license))) => {
                Paragraph::new(license.as_str())
                    .block(block.title(id.as_str()))
                    .wrap(Wrap { trim: true })
                    .scroll((scroll, 0))
                    .render(area, buf);
            }
            Ok(None) => {
                // Either the licence or the package is unavailable
                if let Some(p) = state.get_selected_package() {
                    Paragraph::new(format!(
                        "Licence {} not found for package {}",
                        p.get_uses_license(),
                        p.get_display_name()
                    ))
                    .block(block.title(p.get_uses_license().as_str()))
                    .wrap(Wrap { trim: false })
                    .render(area, buf);
                } else {
                    Paragraph::new("No package selected to view the license")
                        .block(block)
                        .alignment(ratatui::layout::Alignment::Center)
                        .wrap(Wrap { trim: false })
                        .render(area, buf);
                }
            }
            Err(err) => {
                Paragraph::new(err.to_string())
                    .block(block.title("Error loading license"))
                    .alignment(ratatui::layout::Alignment::Center)
                    .wrap(Wrap { trim: false })
                    .render(area, buf);
            }
        }
    }
}
#[derive(Default)]
struct FooterWidget {}
impl StatefulWidget for &FooterWidget {
    type State = AppState;
    fn render(self, area: Rect, buf: &mut ratatui::prelude::Buffer, state: &mut Self::State) {
        let layout = Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).split(area);

        let status_layout =
            Layout::horizontal([Constraint::Length(6), Constraint::Fill(1)]).split(layout[0]);

        Block::new().bg(Color::Gray).render(layout[0], buf);
        let text_style = Style::new().fg(Color::Black);

        // Render the current status
        match state.current_mode {
            Modes::Normal => {
                Line::styled("NORMAL", text_style).render(status_layout[0], buf);
            }
            Modes::FilterInput => {
                Line::styled("FILTER", text_style).render(status_layout[0], buf);
                Line::styled(state.filter_input.as_str(), text_style.fg(Color::Gray))
                    .render(layout[1], buf);
            }
        }
    }
}

#[derive(Default)]
struct DetailsWidget {}

impl StatefulWidget for &DetailsWidget {
    type State = AppState;
    fn render(
        self,
        area: ratatui::prelude::Rect,
        buf: &mut ratatui::prelude::Buffer,
        state: &mut Self::State,
    ) {
        let layout = Layout::new(
            ratatui::layout::Direction::Vertical,
            [
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Fill(1),
            ],
        )
        .split(area);

        let package = if let Some(p) = state.get_selected_package() {
            p
        } else {
            return;
        };

        let version_string = package.get_revision().to_string();

        Paragraph::new(package.get_display_name().as_str())
            .fg(Color::Blue)
            .render(layout[0], buf);

        Line::from(vec![
            Span::styled("version  : ", Style::new().fg(Color::DarkGray)),
            Span::from(version_string),
        ])
        .render(layout[1], buf);

        Line::from(vec![
            Span::styled("path     : ", Style::new().fg(Color::DarkGray)),
            Span::from(package.get_path().as_str()),
        ])
        .render(layout[2], buf);

        Line::from(vec![
            Span::styled("installed: ", Style::new().fg(Color::DarkGray)),
            Span::styled("yes", Style::new().fg(Color::Green)),
        ])
        .render(layout[3], buf);

        // Archive list
        let archive_header = ["host os", "size", "url"]
            .into_iter()
            .map(Cell::from)
            .collect::<Row>()
            .fg(Color::DarkGray)
            .height(1);

        let archive_rows = package
            .get_archives()
            .iter()
            .map(|archive| {
                let platform_cell = Cell::new(archive.get_host_os().as_str());
                let name_cell = Cell::new(archive.get_url().as_str());

                let size_cell = Cell::new(HumanBytes(archive.get_size() as u64).to_string());

                Row::new([platform_cell, size_cell, name_cell])
            })
            .collect::<Vec<Row>>();

        let mut state = TableState::default();

        StatefulWidget::render(
            Table::new(
                archive_rows,
                [
                    Constraint::Length(10),
                    Constraint::Max(12),
                    Constraint::Fill(1),
                ],
            )
            .header(archive_header)
            .block(Block::new().padding(Padding::vertical(1)))
            .highlight_style(Style::new().add_modifier(Modifier::REVERSED)),
            layout[4],
            buf,
            &mut state,
        );
    }
}

#[derive(Default)]
struct AppState {
    /// The selected package
    pub selected_package: usize,

    pub table_state: TableState,

    /// The scroll position on the license page
    pub license_scroll_position: usize,

    pub filter_input: String,
    /// The cursor position for input
    pub filter_input_index: usize,

    /// The current mode
    pub current_mode: Modes,

    /// The filtered packages
    pub filtered_packages: FilteredPackages,

    // caches licenses from sdk path
    licenses: HashMap<String, String>,
}

impl AppState {
    pub fn new(repo: Rc<RepositoryXml>, installed: Rc<HashSet<InstalledPackage>>) -> Self {
        let filtered = FilteredPackages::new(repo, installed);
        Self {
            selected_package: 0,
            license_scroll_position: 0,
            table_state: TableState::default().with_selected(0),
            filter_input: String::new(),
            current_mode: Modes::Normal,
            filter_input_index: 0,
            filtered_packages: filtered,
            licenses: HashMap::new(),
        }
    }
    /// Selects the next package. Wraps around if the end is reached
    pub fn next_package(&mut self) {
        if self.filtered_packages.get_packages().is_empty() {
            self.table_state.select(None);
            return;
        }
        self.selected_package = if self.selected_package.saturating_add(1)
            >= self.filtered_packages.get_packages().len()
        {
            0
        } else {
            self.selected_package.saturating_add(1)
        };

        self.table_state.select(Some(self.selected_package));
    }

    /// Selects the previous package. Wraps around if the beginning is reached.
    pub fn previous_package(&mut self) {
        if self.filtered_packages.get_packages().is_empty() {
            self.table_state.select(None);
            return;
        }
        self.selected_package = if self.selected_package == 0 {
            self.filtered_packages
                .get_packages()
                .len()
                .saturating_sub(1)
        } else {
            self.selected_package.saturating_sub(1)
        };

        self.table_state.select(Some(self.selected_package));
    }

    /// Returns the main table state
    // pub fn get_main_table_state(&mut self) -> &mut TableState {
    //     &mut self.table_state
    // }

    /// Returns the selected index
    // pub fn get_selected_package_index(&self) -> usize {
    //     self.selected_package
    // }
    /// Returns the selected package
    pub fn get_selected_package(&self) -> Option<&RemotePackage> {
        self.filtered_packages
            .get_packages()
            .get(self.selected_package)
    }
    /// Returns remote packages from repo.
    /// Applies filter if it was activated
    pub fn get_remote_packages(&self) -> &Vec<RemotePackage> {
        self.filtered_packages.get_packages()
    }
    /// Returns the license for current package
    pub fn get_selected_license(&mut self) -> anyhow::Result<Option<(String, &String)>> {
        // Should fix this clone
        if let Some(package) = self.get_selected_package().cloned() {
            let id = package.get_uses_license();
            self.load_license(id)
                .map(|l| l.map(|license| (id.to_string(), license)))
        } else {
            Ok(None)
        }
    }
    /// Moves the input cursor left
    pub fn move_cursor_left(&mut self) {
        self.filter_input_index = self.filter_input_index.saturating_sub(1);
    }
    /// Moves the input cursor right
    pub fn move_cursor_right(&mut self) {
        let new_index = self.filter_input_index.saturating_add(1);
        self.filter_input_index = new_index.clamp(0, self.filter_input.chars().count());
    }
    /// Deletes  characters in the input posirion
    pub fn backspace_cursor(&mut self) {
        if self.filter_input_index != 0 {
            let current_index = self.filter_input_index;
            let left_characters = self.filter_input.chars().take(current_index - 1);
            let right_characters = self.filter_input.chars().skip(current_index);

            self.filter_input = left_characters.chain(right_characters).collect();
            self.move_cursor_left();
        }
        self.update_filter();
    }
    /// Inserts the character at cursor position
    pub fn insert_at_cursor(&mut self, c: char) {
        let index = self
            .filter_input
            .char_indices()
            .map(|(i, _)| i)
            .nth(self.filter_input_index)
            .unwrap_or(self.filter_input.len());

        self.filter_input.insert(index, c);
        self.move_cursor_right();
        self.update_filter();
    }
    fn update_filter(&mut self) {
        self.filtered_packages.pop_filter();
        self.filtered_packages
            .push_filter(SdkFilters::Name(self.filter_input.clone()));
        self.filtered_packages.apply();
    }
    /// Fetches license from sdkpath
    fn load_license(&mut self, id: &str) -> anyhow::Result<Option<&String>> {
        if self.licenses.contains_key(id) {
            return Ok(self.licenses.get(id));
        }

        let mut sdk = get_home().context("Failed to get LABt home while fetching licenses")?;
        sdk.push("sdk");
        sdk.push("licenses");
        sdk.push(id);

        if !sdk.exists() {
            bail!("{} does not exists in stored licenses", id);
        }
        let mut file = File::open(&sdk).context(format!("Failed to open license file: {}, from LABt home. Consider force updating repository list with --update-repository-list.", sdk.to_string_lossy()))?;
        let mut license = String::new();
        file.read_to_string(&mut license)?;

        self.licenses.insert(id.to_string(), license);

        Ok(self.licenses.get(id))
    }

    /// Tries to install the selected package
    fn install_current(&mut self) {
        let _package = if let Some(p) = self.get_selected_package() {
            p
        } else {
            return;
        };
        // TODO call install logic
    }
}

mod help_pages {
    pub const MAIN: &str = "main list";
    pub const LICENSE: &str = "license page";
    pub const HELP: &str = "help page";
}

#[derive(Default)]
pub struct SdkManager {
    exit: bool,

    current_page: Pages,

    state: AppState,
    show_help: bool,

    help_popup: HelpPopoup,
}

impl SdkManager {
    pub fn new(repo: Rc<RepositoryXml>, installed: Rc<HashSet<InstalledPackage>>) -> Self {
        let state = AppState::new(repo, installed);
        SdkManager {
            exit: false,
            current_page: Pages::MainList,
            state,
            show_help: false,
            help_popup: HelpPopoup::new(80, 80),
        }
    }
    pub fn run(&mut self, terminal: &mut Tui) -> io::Result<()> {
        self.load_help();
        while !self.exit {
            terminal.draw(|frame| {
                self.render_frame(frame);
            })?;
            self.handle_events()?;
        }
        Ok(())
    }
    pub fn load_help(&mut self) {
        self.help_popup.set_help(
            help_pages::MAIN.to_string(),
            vec![
                HelpEntry::new("/", "Filter"),
                HelpEntry::new("?", "Help"),
                HelpEntry::new("Enter", "Select Entry"),
                HelpEntry::new("Up/Down", "Scroll entries"),
                HelpEntry::new("L", "License"),
                HelpEntry::new("I", "Install"),
            ],
        );
        self.help_popup.set_help(
            help_pages::LICENSE.to_string(),
            vec![
                HelpEntry::new("Enter", "Accept licence"),
                HelpEntry::new("Up/Down", "Scroll text"),
                HelpEntry::new("Esc", "Back/Cancel"),
            ],
        );
        self.help_popup.set_help(
            help_pages::HELP.to_string(),
            vec![
                HelpEntry::new("Enter", "Accept licence"),
                HelpEntry::new("Up/Down", "Scroll text"),
                HelpEntry::new("Esc/q/?", "Close this menu"),
            ],
        );
    }
    fn render_frame(&mut self, frame: &mut Frame) {
        // frame.render_widget(self, frame.size());
        let layout = Layout::new(
            ratatui::layout::Direction::Vertical,
            [
                Constraint::Fill(1),
                Constraint::Length(2),
                Constraint::Length(2),
            ],
        )
        .split(frame.size());

        match self.current_page {
            Pages::MainList => {
                frame.render_stateful_widget(&MainListPage::default(), layout[0], &mut self.state);
                if let Some(help) = self.help_popup.help.get_mut(help_pages::MAIN) {
                    frame.render_stateful_widget(HelpFooter::default(), layout[1], help);
                }
            }
            Pages::License => {
                frame.render_stateful_widget(&LicensePage::default(), layout[0], &mut self.state);
                if let Some(help) = self.help_popup.help.get_mut(help_pages::LICENSE) {
                    frame.render_stateful_widget(HelpFooter::default(), layout[1], help);
                }
            }
        }
        frame.render_stateful_widget(&FooterWidget::default(), layout[2], &mut self.state);
        if matches!(self.state.current_mode, Modes::FilterInput) {
            frame.set_cursor(
                layout[2].x + self.state.filter_input_index as u16,
                layout[2].y + 1,
            );
        }

        if self.show_help {
            self.help_popup.draw(frame);
        }
    }
    fn handle_events(&mut self) -> io::Result<()> {
        // if event::poll(Duration::from_millis(16))? {
        if let Event::Key(key) = event::read()? {
            if key.kind == KeyEventKind::Press {
                match self.state.current_mode {
                    Modes::Normal if self.show_help => match key.code {
                        KeyCode::Up => {
                            self.help_popup.scroll_position =
                                self.help_popup.scroll_position.saturating_sub(1);
                        }
                        KeyCode::Down => {
                            self.help_popup.scroll_position =
                                self.help_popup.scroll_position.saturating_add(1);
                        }
                        KeyCode::Char('q') | KeyCode::Char('?') | KeyCode::Esc => {
                            self.show_help = false;
                        }
                        _ => {}
                    },
                    Modes::Normal => match key.code {
                        // Up scroll movements
                        KeyCode::Up => match self.current_page {
                            Pages::MainList => self.state.previous_package(),
                            Pages::License => {
                                self.state.license_scroll_position =
                                    self.state.license_scroll_position.saturating_sub(2);
                            } // _ => {}
                        },

                        // Down scroll movements
                        KeyCode::Down => match self.current_page {
                            Pages::MainList => self.state.next_package(),
                            Pages::License => {
                                self.state.license_scroll_position =
                                    self.state.license_scroll_position.saturating_add(2);
                            } // _ => {}
                        },
                        // Help
                        KeyCode::Char('?') => {
                            self.show_help = true;
                        }

                        // Quit
                        KeyCode::Char('q') => self.exit = true,
                        KeyCode::Char('L') if matches!(self.current_page, Pages::MainList) => {
                            self.current_page = Pages::License;
                        }
                        KeyCode::Esc if matches!(self.current_page, Pages::License) => {
                            self.current_page = Pages::MainList;
                        }
                        KeyCode::Esc if self.show_help => {
                            self.show_help = false;
                        }
                        KeyCode::Char('/') if matches!(self.current_page, Pages::MainList) => {
                            self.state.current_mode = Modes::FilterInput;
                            if !self.state.filtered_packages.has_filters() {
                                self.state
                                    .filtered_packages
                                    .push_filter(SdkFilters::Name(String::new()));
                                // self.state.filtered_packages.apply();
                            }
                        }
                        // Install
                        KeyCode::Char('I') => {
                            self.state.install_current();
                        }
                        _ => {}
                    },
                    Modes::FilterInput => match key.code {
                        KeyCode::Esc => {
                            self.state.current_mode = Modes::Normal;
                            self.state.filtered_packages.pop_filter();
                            self.state.filtered_packages.apply();
                        }
                        KeyCode::Enter => {
                            self.state.current_mode = Modes::Normal;
                            self.state.filtered_packages.apply();
                        }
                        KeyCode::Backspace => {
                            self.state.backspace_cursor();
                        }
                        KeyCode::Left => {
                            self.state.move_cursor_left();
                        }
                        KeyCode::Right => {
                            self.state.move_cursor_right();
                        }
                        KeyCode::Char(c) => {
                            self.state.insert_at_cursor(c);
                        }
                        _ => {}
                    },
                }
            }
            // }
        }
        Ok(())
    }
}
