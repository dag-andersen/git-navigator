use ratatui::layout::{Constraint, Layout, Rect};

use crate::app::{Focus, PanelLayout};

pub(crate) fn panel_areas(
    area: Rect,
    focus: Focus,
    expanded: bool,
    panel_layout: PanelLayout,
    show_worktrees: bool,
) -> [Rect; 3] {
    if !show_worktrees {
        if expanded {
            let [files, diff] = match focus {
                Focus::Diff => {
                    Layout::horizontal([Constraint::Length(5), Constraint::Fill(1)]).areas(area)
                }
                Focus::Files | Focus::Worktrees => {
                    Layout::horizontal([Constraint::Fill(1), Constraint::Length(5)]).areas(area)
                }
            };
            return [Rect::default(), files, diff];
        }

        return match panel_layout {
            PanelLayout::Columns | PanelLayout::SidebarLeft => {
                let [files, diff] =
                    Layout::horizontal([Constraint::Percentage(25), Constraint::Percentage(75)])
                        .areas(area);
                [Rect::default(), files, diff]
            }
            PanelLayout::SidebarTop => {
                let [files, diff] =
                    Layout::vertical([Constraint::Percentage(25), Constraint::Percentage(75)])
                        .areas(area);
                [Rect::default(), files, diff]
            }
        };
    }

    if expanded {
        let constraints = match focus {
            Focus::Worktrees => [
                Constraint::Fill(1),
                Constraint::Length(5),
                Constraint::Length(5),
            ],
            Focus::Files => [
                Constraint::Length(5),
                Constraint::Fill(1),
                Constraint::Length(5),
            ],
            Focus::Diff => [
                Constraint::Length(5),
                Constraint::Length(5),
                Constraint::Fill(1),
            ],
        };
        return Layout::horizontal(constraints).areas(area);
    }

    match panel_layout {
        PanelLayout::Columns => Layout::horizontal([
            Constraint::Percentage(18),
            Constraint::Percentage(22),
            Constraint::Percentage(60),
        ])
        .areas(area),
        PanelLayout::SidebarLeft => {
            let [sidebar, diff] =
                Layout::horizontal([Constraint::Percentage(25), Constraint::Percentage(75)])
                    .areas(area);
            let [worktrees, files] =
                Layout::vertical([Constraint::Percentage(35), Constraint::Percentage(65)])
                    .areas(sidebar);
            [worktrees, files, diff]
        }
        PanelLayout::SidebarTop => {
            let [top, diff] =
                Layout::vertical([Constraint::Percentage(25), Constraint::Percentage(75)])
                    .areas(area);
            let [worktrees, files] =
                Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
                    .areas(top);
            [worktrees, files, diff]
        }
    }
}
