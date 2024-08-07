use std::{
    collections::HashMap,
    fs::File,
    io::{self, Read},
    marker::PhantomData,
};

use anyhow::{bail, Context};
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use indicatif::HumanBytes;
use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span},
    widgets::{
        Block, Borders, Cell, Clear, List, ListState, Padding, Paragraph, Row, Scrollbar,
        ScrollbarOrientation, ScrollbarState, StatefulWidget, Table, TableState, Widget, Wrap,
    },
    Frame,
};

use crate::{
    config::repository::{ChannelType, RemotePackage},
    get_home,
    submodules::{
        sdk::InstalledPackage,
        sdkmanager::filters::{FilteredPackages, SdkFilters},
    },
};

use super::Tui;

const AVAILABLE_CHANNELS: &[ChannelType] = &[
    ChannelType::Stable,
    ChannelType::Beta,
    ChannelType::Dev,
    ChannelType::Canary,
];

/// List of pages we can switch between
#[derive(Default)]
enum Pages {
    #[default]
    MainList,
    License,
    Details,
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
    percent_width: u8,
    percent_height: u8,
    scroll_position: u16,
    help: HashMap<String, Vec<HelpEntry>>,
}

impl HelpPopoup {
    pub fn new(percent_width: u8, percent_height: u8) -> Self {
        Self {
            percent_width,
            percent_height,
            scroll_position: 0,
            help: HashMap::new(),
        }
    }
    pub fn set_help(&mut self, context: String, entries: Vec<HelpEntry>) {
        self.help.insert(context, entries);
    }
    pub fn draw(&mut self, frame: &mut Frame) {
        let area = calculate_center_area(self.percent_width, self.percent_height, frame.size());
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
struct MainListPage<'a> {
    _phantom: PhantomData<&'a ()>,
}

impl<'a> StatefulWidget for &MainListPage<'a> {
    type State = AppState<'a>;
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

        let list_layout =
            Layout::horizontal([Constraint::Fill(1), Constraint::Length(1)]).split(layout[1]);
        // page title
        if state
            .filtered_packages
            .single_filters
            .contains(&SdkFilters::Installed)
        {
            Paragraph::new("Installed packages").render(layout[0], buf);
        } else {
            Paragraph::new("Available packages").render(layout[0], buf);
        }

        let header_style = Style::new().fg(Color::DarkGray).underlined();
        let header = ["", "Name", "Version", "Path"]
            .into_iter()
            .map(Cell::from)
            .collect::<Row>()
            .style(header_style)
            .height(1);
        let mut longest_version_string = 7; // default value equal to "version".len()
        let packages = state.get_remote_packages();
        let packages_count = packages.len();
        let scroll = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .track_symbol(None)
            .begin_symbol(None)
            .thumb_symbol(ratatui::symbols::block::HALF)
            .end_symbol(None);
        let mut scroll_state = ScrollbarState::new(packages_count)
            .position(state.selected_package.clamp(0, packages_count));

        let rows: Vec<Row> = packages
            .iter()
            .map(|package| {
                let name_cell = Cell::new(package.get_display_name().as_str()).style(Style::new());

                let revision = package.get_revision();
                let version_string = revision.to_string();
                if version_string.len() > longest_version_string {
                    longest_version_string = version_string.len();
                }
                let version_cell = Cell::new(version_string);
                let path = Cell::new(package.get_path().as_str());

                if let Some(action) = state.pending_actions.get(package) {
                    let mut cells = vec![
                        Cell::new(ratatui::symbols::DOT).bold(),
                        name_cell,
                        version_cell,
                        path,
                    ];
                    match action {
                        PendingAction::Install => Row::new(cells).fg(Color::Green),
                        PendingAction::Uninstall => Row::new(cells).fg(Color::LightRed),
                        _ => {
                            cells[0] = Cell::new("");
                            cells[1] = cells[1].clone().fg(Color::Cyan);
                            cells[2] = cells[2].clone().fg(Color::DarkGray);
                            Row::new(cells)
                        }
                    }
                } else {
                    Row::new(vec![
                        Cell::new(""),
                        name_cell.fg(Color::Cyan),
                        version_cell.fg(Color::DarkGray),
                        path,
                    ])
                }
            })
            .collect();

        let table = Table::new(
            rows,
            [
                Constraint::Length(1),
                Constraint::Fill(2),
                Constraint::Length(longest_version_string as u16),
                Constraint::Fill(2),
            ],
        )
        .header(header)
        .highlight_symbol(">")
        .column_spacing(1);
        ratatui::widgets::StatefulWidget::render(
            table,
            list_layout[0],
            buf,
            &mut state.table_state.clone(),
        );
        ratatui::widgets::StatefulWidget::render(scroll, list_layout[1], buf, &mut scroll_state);
        let details = DetailsWidget::default();
        let inner = layout[2].inner(&ratatui::layout::Margin {
            horizontal: 3,
            vertical: 1,
        });

        let block = Block::new().borders(Borders::TOP);
        block.render(layout[2], buf);
        StatefulWidget::render(&details, inner, buf, state);
    }
}

#[derive(Default)]
struct LicensePage<'a> {
    _phantom: PhantomData<&'a ()>,
}

impl<'a> StatefulWidget for &LicensePage<'a> {
    type State = AppState<'a>;
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
struct FooterWidget<'a> {
    _phantom: PhantomData<&'a ()>,
}
impl<'a> StatefulWidget for &FooterWidget<'a> {
    type State = AppState<'a>;
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
                let mut filter_list: Vec<String> = Vec::new();
                for filter in &state.filtered_packages.filters {
                    match filter {
                        SdkFilters::Name(name) if !name.is_empty() => {
                            filter_list.push(format!("/{}", name));
                        }
                        SdkFilters::Version(version) => {
                            filter_list.push(format!("v{}", version));
                        }
                        _ => {}
                    }
                }
                Line::styled(filter_list.join(" & "), text_style.fg(Color::Gray))
                    .render(layout[1], buf);
            }
            Modes::FilterInput => {
                Line::styled("FILTER", text_style).render(status_layout[0], buf);
                Line::styled(state.filter_input.as_str(), text_style.fg(Color::Gray))
                    .render(layout[1], buf);
            }
        }

        let mut filters: Line = Line::default();

        if let Some(channel) = &state.filtered_packages.get_channel() {
            filters.push_span(Span::styled(
                format!("{} | ", channel.to_string().to_uppercase()),
                text_style,
            ));
        } else {
            filters.push_span(Span::styled("AC | ", text_style));
        }

        if state
            .filtered_packages
            .single_filters
            .contains(&SdkFilters::Installed)
        {
            filters.push_span(Span::styled("IN | ", text_style))
        }

        if state
            .filtered_packages
            .single_filters
            .contains(&SdkFilters::Obsolete(false))
        {
            filters.push_span(Span::styled("HO | ", text_style))
        } else {
            filters.push_span(Span::styled("SO | ", text_style))
        }

        filters.push_span(Span::styled(
            format!(
                "{}/{}",
                state.selected_package.saturating_add(1),
                state.filtered_packages.get_packages().len()
            ),
            text_style,
        ));

        filters.right_aligned().render(status_layout[1], buf);
    }
}

#[derive(Default)]
struct DetailsWidget<'a> {
    _phantom: PhantomData<&'a ()>,
}

impl<'a> StatefulWidget for &DetailsWidget<'a> {
    type State = AppState<'a>;
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

        if package.is_obsolete() {
            Paragraph::new(Line::from(vec![
                Span::styled(
                    package.get_display_name().as_str(),
                    Style::new().fg(Color::Blue),
                ),
                Span::styled(" (obsolete)", Style::new().fg(Color::Yellow)),
            ]))
            .render(layout[0], buf);
        } else {
            Paragraph::new(package.get_display_name().as_str())
                .fg(Color::Blue)
                .render(layout[0], buf);
        }

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
        let channel = package.get_channel();

        Line::from(vec![
            Span::styled("channel  : ", Style::new().fg(Color::DarkGray)),
            Span::from(channel.to_string()),
        ])
        .render(layout[3], buf);

        // Check if a package is installed
        if state
            .filtered_packages
            .installed
            .contains_id(&InstalledPackage::new(
                package.get_path().clone(),
                package.get_revision().clone(),
                channel.clone(),
            ))
            .is_some()
        {
            Line::from(vec![
                Span::styled("installed: ", Style::new().fg(Color::DarkGray)),
                Span::styled("yes", Style::new().fg(Color::Green)),
            ])
            .render(layout[4], buf);
        } else {
            Line::from(vec![
                Span::styled("installed: ", Style::new().fg(Color::DarkGray)),
                Span::styled("no", Style::new().fg(Color::Red)),
            ])
            .render(layout[4], buf);
        }

        // Archive list
        let archive_header = ["host os", "bit", "size", "url"]
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
                let bit_cell = Cell::new(match archive.get_host_bits() {
                    crate::config::repository::BitSizeType::Bit64 => "64",
                    crate::config::repository::BitSizeType::Bit32 => "32",
                    crate::config::repository::BitSizeType::Unset => " - ",
                });
                let name_cell = Cell::new(archive.get_url().as_str());

                let size_cell = Cell::new(HumanBytes(archive.get_size() as u64).to_string());

                Row::new([platform_cell, bit_cell, size_cell, name_cell])
            })
            .collect::<Vec<Row>>();

        let mut state = TableState::default();

        StatefulWidget::render(
            Table::new(
                archive_rows,
                [
                    Constraint::Length(10),
                    Constraint::Length(3),
                    Constraint::Max(12),
                    Constraint::Fill(1),
                ],
            )
            .header(archive_header)
            .block(Block::new().padding(Padding::vertical(1)))
            .highlight_style(Style::new().add_modifier(Modifier::REVERSED)),
            layout[5],
            buf,
            &mut state,
        );
    }
}

type PendingActions = HashMap<RemotePackage, PendingAction>;

pub struct ConfirmActionPopup<'a> {
    actions: &'a PendingActions,
}
impl<'a> ConfirmActionPopup<'a> {
    pub fn new(actions: &'a PendingActions) -> Self {
        Self { actions }
    }
}
impl<'a> Widget for ConfirmActionPopup<'a> {
    fn render(self, area: Rect, buf: &mut ratatui::prelude::Buffer)
    where
        Self: Sized,
    {
        let inner = area.inner(&ratatui::layout::Margin {
            horizontal: 2,
            vertical: 2,
        });
        Clear.render(area, buf);
        Block::bordered()
            .title("Confirm changes")
            .title_bottom(Line::from("(Esc) Cancel").left_aligned())
            .title_bottom(Line::from("(Enter) Confirm").right_aligned())
            .render(area, buf);
        let layout = Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]).split(inner);

        Paragraph::new("You are about to:")
            .bold()
            .render(layout[0], buf);
        let mut installs: Vec<&RemotePackage> = Vec::new();
        let mut uninstalls: Vec<&RemotePackage> = Vec::new();

        for (package, action) in self.actions.iter() {
            match action {
                PendingAction::Install => installs.push(package),
                PendingAction::Uninstall => uninstalls.push(package),
                _ => {}
            }
        }

        let mut lines: Vec<Line> = Vec::new();
        // add consistency in list
        installs.sort_unstable_by_key(|p| p.get_display_name());
        uninstalls.sort_unstable_by_key(|p| p.get_display_name());
        lines.push(Line::raw(""));
        lines.push("Install".underlined().bold().into());

        for package in installs {
            lines.push(Line::from(vec![
                Span::from("[+] ").green(),
                Span::from(format!(
                    "{} {} v{}",
                    package.get_display_name(),
                    package.get_path(),
                    package.get_revision()
                )),
            ]));
        }
        lines.push(Line::raw(""));
        lines.push("Uninstall".underlined().bold().into());
        for package in uninstalls {
            lines.push(Line::from(vec![
                Span::from("[-] ").red(),
                Span::from(format!(
                    "{} {} v{}",
                    package.get_display_name(),
                    package.get_path(),
                    package.get_revision()
                )),
            ]));
        }

        Paragraph::new(lines)
            .wrap(Wrap { trim: true })
            .render(layout[1], buf);
    }
}

#[derive(Debug, Default)]
pub enum PendingAction {
    #[default]
    /// Dont do anything
    Noop,
    /// Install package
    Install,
    /// Uninstall package
    Uninstall,
}

struct AppState<'a> {
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
    pub filtered_packages: &'a mut FilteredPackages<'a, 'a>,

    // caches licenses from sdk path
    licenses: HashMap<String, String>,

    /// Render full details
    pub show_full_details: bool,
    /// The pending actions to perform
    pub pending_actions: HashMap<RemotePackage, PendingAction>,
}

impl<'a> AppState<'a> {
    pub fn new(packages: &'a mut FilteredPackages<'a, 'a>) -> Self {
        Self {
            selected_package: 0,
            license_scroll_position: 0,
            table_state: TableState::default().with_selected(0),
            filter_input: String::new(),
            current_mode: Modes::Normal,
            filter_input_index: 0,
            filtered_packages: packages,
            licenses: HashMap::new(),
            show_full_details: false,
            pending_actions: HashMap::new(),
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
            .copied()
    }
    /// Returns remote packages from repo.
    /// Applies filter if it was activated
    pub fn get_remote_packages(&self) -> &Vec<&RemotePackage> {
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
    /// Sets a pending action for a particular package
    pub fn set_action(&mut self, package: RemotePackage, action: PendingAction) {
        self.pending_actions.insert(package, action);
    }
    /// Toggles the action to be performed on select package.
    /// If package is installed sets action to uninstall
    /// If package is not installed it sets the action to install
    /// Repeating the action sets it to Noop
    /// Does nothing if there is no action set
    pub fn toggle_action(&mut self) {
        let Some(package) = self.get_selected_package() else {
            return;
        };

        let installed = InstalledPackage::new(
            package.get_path().to_string(),
            package.get_revision().to_owned(),
            package.get_channel().clone(),
        );

        if self
            .filtered_packages
            .installed
            .contains_id(&installed)
            .is_some()
        {
            // it is installed
            let package = package.clone();
            if let Some(action) = self.pending_actions.get_mut(&package) {
                match action {
                    PendingAction::Noop => *action = PendingAction::Uninstall,
                    _ => *action = PendingAction::Noop,
                }
            } else {
                self.set_action(package, PendingAction::Uninstall);
            }
        } else {
            // not installed
            let package = package.clone();
            if let Some(action) = self.pending_actions.get_mut(&package) {
                match action {
                    PendingAction::Noop => *action = PendingAction::Install,
                    _ => *action = PendingAction::Noop,
                }
            } else {
                self.set_action(package, PendingAction::Install);
            }
        }
    }
}

mod help_pages {
    pub const MAIN: &str = "main list";
    pub const LICENSE: &str = "license page";
    pub const HELP: &str = "help page";
    pub const DETAILS: &str = "package details";
}

pub struct SdkManager<'a> {
    exit: bool,

    current_page: Pages,

    state: AppState<'a>,
    show_help: bool,

    help_popup: HelpPopoup,
    show_channel_list: bool,

    channels: Vec<String>,
    channels_list_state: ListState,

    show_exit_dialog: bool,
}

impl<'a> SdkManager<'a> {
    pub fn new(packages: &'a mut FilteredPackages<'a, 'a>) -> Self {
        let mut channel_state = ListState::default();
        let mut channels: Vec<String> = AVAILABLE_CHANNELS.iter().map(|c| c.to_string()).collect();
        channel_state.select(Some(channels.len()));
        channels.push("ALL".to_string());

        let state = AppState::new(packages);

        SdkManager {
            exit: false,
            current_page: Pages::MainList,
            state,
            show_help: false,
            help_popup: HelpPopoup::new(80, 80),
            show_channel_list: false,
            channels,
            channels_list_state: channel_state,
            show_exit_dialog: false,
        }
    }
    /// ===============
    ///  Entry point
    /// ===============
    /// Starts rendering sdkmanager tui and listening for key events
    pub fn run(mut self, terminal: &mut Tui) -> io::Result<HashMap<RemotePackage, PendingAction>> {
        self.load_help();
        while !self.exit {
            terminal.draw(|frame| {
                self.render_frame(frame);
            })?;
            self.handle_events()?;
        }
        Ok(self.state.pending_actions)
    }
    /// Loads help popup with common help messages
    pub fn load_help(&mut self) {
        self.help_popup.set_help(
            help_pages::MAIN.to_string(),
            vec![
                HelpEntry::new("/", "Search"),
                HelpEntry::new("?", "Help"),
                HelpEntry::new("Space", "Select for (un)install"),
                HelpEntry::new("Enter", "Save changes"),
                HelpEntry::new("Up/Down", "Scroll entries"),
                HelpEntry::new("L", "License"),
                HelpEntry::new("i", "Show installed"),
                HelpEntry::new("o", "Show obsolete"),
                HelpEntry::new("c", "Select Channel"),
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
        self.help_popup.set_help(
            help_pages::DETAILS.to_string(),
            vec![
                HelpEntry::new("Up/Down", "Scroll"),
                HelpEntry::new("Esc", "Back/Cancel"),
                HelpEntry::new("L", "License"),
            ],
        );
    }
    /// Call draw for current frame
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
                let spans: Vec<Span> = vec![
                    "Press ".into(),
                    "[Space]".fg(Color::DarkGray),
                    " to mark packages for install/uninstall. Press ".into(),
                    "[Enter] ".fg(Color::DarkGray),
                    "to save changes. ".into(),
                    "[?]".fg(Color::DarkGray),
                    " for more help. ".into(),
                    "[q]".fg(Color::DarkGray),
                    " to quit.".into(),
                ];
                frame.render_widget(
                    Paragraph::new(Line::from(spans)).wrap(Wrap { trim: true }),
                    layout[1],
                );
            }
            Pages::License => {
                frame.render_stateful_widget(&LicensePage::default(), layout[0], &mut self.state);
                if let Some(help) = self.help_popup.help.get_mut(help_pages::LICENSE) {
                    frame.render_stateful_widget(HelpFooter::default(), layout[1], help);
                }
            }
            Pages::Details => {
                frame.render_stateful_widget(&DetailsWidget::default(), layout[0], &mut self.state);
                if let Some(help) = self.help_popup.help.get_mut(help_pages::DETAILS) {
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

        if self.show_channel_list {
            // render channel list
            let count = self.channels.len();
            let area = Rect::new(
                layout[2].x,
                layout[2].y.saturating_sub((count + 2) as u16),
                20,
                (count + 2) as u16,
            );
            frame.render_widget(Clear, area);

            let list: List = List::new(
                self.channels
                    .iter()
                    .map(|f| f.as_str())
                    .collect::<Vec<&str>>(),
            );

            let list = list
                .block(Block::bordered())
                .highlight_symbol(">")
                .highlight_style(Style::new().add_modifier(Modifier::REVERSED));

            frame.render_stateful_widget(list, area, &mut self.channels_list_state);
        }
        if self.show_exit_dialog {
            frame.render_widget(
                ConfirmActionPopup::new(&self.state.pending_actions),
                calculate_center_area(100, 100, frame.size()),
            );
        }
    }
    /// Blocks to read for any input event to the console.
    fn handle_events(&mut self) -> io::Result<()> {
        // if event::poll(Duration::from_millis(16))? {
        if let Event::Key(key) = event::read()? {
            if key.kind == KeyEventKind::Press {
                match self.state.current_mode {
                    Modes::Normal if self.show_exit_dialog => match key.code {
                        KeyCode::Enter => self.exit = true,
                        KeyCode::Esc | KeyCode::Char('q') => self.show_exit_dialog = false,
                        _ => {}
                    },
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
                    Modes::Normal if self.show_channel_list => match key.code {
                        KeyCode::Up => {
                            if let Some(index) = self.channels_list_state.selected() {
                                if index == 0 {
                                    self.channels_list_state
                                        .select(Some(self.channels.len().saturating_sub(1)));
                                } else {
                                    self.channels_list_state
                                        .select(Some(index.saturating_sub(1)))
                                }
                            } else if !self.channels.is_empty() {
                                self.channels_list_state
                                    .select(Some(self.channels.len().saturating_sub(1)));
                            }
                        }
                        KeyCode::Down => {
                            if let Some(index) = self.channels_list_state.selected() {
                                if index.saturating_add(1) == self.channels.len() {
                                    self.channels_list_state.select(Some(0));
                                } else {
                                    self.channels_list_state
                                        .select(Some(index.saturating_add(1)));
                                }
                            } else if !self.channels.is_empty() {
                                self.channels_list_state.select(Some(0));
                            }
                        }
                        KeyCode::Enter => {
                            if let Some(index) = self.channels_list_state.selected() {
                                if let Some(channel) = self.channels.get(index) {
                                    if channel == "ALL" {
                                        // clear the channel flags
                                        self.state.filtered_packages.set_channel(None);
                                    } else {
                                        let channel: ChannelType = channel.to_owned().into();
                                        self.state.filtered_packages.set_channel(Some(channel));
                                    }
                                }
                            }
                            self.state.filtered_packages.apply();
                            self.show_channel_list = false;
                        }
                        KeyCode::Char('c') | KeyCode::Esc => {
                            self.show_channel_list = false;
                        }
                        _ => {}
                    },
                    Modes::Normal => match key.code {
                        // open details page
                        KeyCode::Enter => {
                            if !self.state.pending_actions.is_empty() {
                                self.show_exit_dialog = true;
                            } else {
                                self.exit = true;
                            }
                        }
                        // Up scroll movements
                        KeyCode::Up => match self.current_page {
                            Pages::MainList => self.state.previous_package(),
                            Pages::License => {
                                self.state.license_scroll_position =
                                    self.state.license_scroll_position.saturating_sub(2);
                            }
                            _ => {}
                        },

                        // Down scroll movements
                        KeyCode::Down => match self.current_page {
                            Pages::MainList => self.state.next_package(),
                            Pages::License => {
                                self.state.license_scroll_position =
                                    self.state.license_scroll_position.saturating_add(2);
                            }
                            _ => {}
                        },
                        // Help
                        KeyCode::Char('?') => {
                            self.show_help = true;
                        }

                        // Quit
                        KeyCode::Char('q') => {
                            self.state.pending_actions.clear();
                            self.exit = true;
                        }
                        KeyCode::Char('L')
                            if matches!(self.current_page, Pages::MainList | Pages::Details) =>
                        {
                            self.current_page = Pages::License;
                        }
                        KeyCode::Esc if self.show_help => {
                            self.show_help = false;
                        }
                        KeyCode::Esc => match self.current_page {
                            Pages::Details => {
                                self.current_page = Pages::MainList;
                                self.state.show_full_details = false;
                            }
                            Pages::License if self.state.show_full_details => {
                                self.current_page = Pages::Details;
                            }
                            Pages::License => self.current_page = Pages::MainList,
                            _ => {}
                        },
                        KeyCode::Char('/') if matches!(self.current_page, Pages::MainList) => {
                            self.state.current_mode = Modes::FilterInput;
                            if !self.state.filtered_packages.has_filters() {
                                self.state
                                    .filtered_packages
                                    .push_filter(SdkFilters::Name(String::new()));
                                // self.state.filtered_packages.apply();
                            }
                        }
                        // Filter by installed
                        KeyCode::Char('i') => {
                            if self
                                .state
                                .filtered_packages
                                .single_filters
                                .contains(&SdkFilters::Installed)
                            {
                                self.state
                                    .filtered_packages
                                    .remove_singleton_filter(&SdkFilters::Installed);
                            } else {
                                self.state
                                    .filtered_packages
                                    .insert_singleton_filter(SdkFilters::Installed);
                            }
                            self.state.filtered_packages.apply();
                        }
                        KeyCode::Char('o') => {
                            if self
                                .state
                                .filtered_packages
                                .single_filters
                                .contains(&SdkFilters::Obsolete(false))
                            {
                                self.state
                                    .filtered_packages
                                    .remove_singleton_filter(&SdkFilters::Obsolete(false));
                            } else {
                                self.state
                                    .filtered_packages
                                    .insert_singleton_filter(SdkFilters::Obsolete(false));
                            }
                        }
                        KeyCode::Char('c') => {
                            self.show_channel_list = true;
                        }
                        KeyCode::Char(' ') => {
                            self.state.toggle_action();
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
/// Caclulates the center region based on preferred width and height percentages
pub fn calculate_center_area(percentage_width: u8, percentage_height: u8, area: Rect) -> Rect {
    let percent_width = percentage_width.clamp(0, 100) as u16;
    let percent_height = percentage_height.clamp(0, 100) as u16;

    // Calculate the position of the popup based on width&height
    let center_vertical = Layout::vertical([
        Constraint::Percentage((100 - percent_height) / 2),
        Constraint::Percentage(percent_height),
        Constraint::Percentage((100 - percent_height) / 2),
    ])
    .split(area);
    let center_horizontal = Layout::horizontal([
        Constraint::Percentage((100 - percent_width) / 2),
        Constraint::Percentage(percent_width),
        Constraint::Percentage((100 - percent_width) / 3),
    ])
    .split(center_vertical[1]);

    center_horizontal[1]
}
