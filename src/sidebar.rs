// Copyright 2023 System76 <info@system76.com>
// SPDX-License-Identifier: GPL-3.0-only

//! WMDE: the grouped sidebar, shared by the main window and the file-chooser dialog.
//!
//! The layout follows ref/files-sidebar.png (Nemo): bold group headers instead of divider
//! lines, full-colour icons, and one row per entry. The widgets are generic over the host
//! application's message type and take the messages they should emit as parameters, because
//! the two callers wire the same rows to different actions (the dialog has no tabs, so no
//! middle-click, and it ejects through its own message).

use cosmic::iced::{Alignment, Length};
use cosmic::widget::{self, icon};
use cosmic::{Element, cosmic_theme, theme};

use crate::mouse_area::MouseArea;

/// Look up a sidebar icon by name.
///
/// `prefer_svg` is what makes this different from a plain `icon::from_name`, and it is load
/// bearing: the lookup sorts scalable directories ahead of every fixed-size one BEFORE it
/// compares sizes (cosmic-freedesktop-icons, `closest_match_size`), so it decides between
/// Qogir's two drawing styles. `scalable/` is the full-colour art; `16/` is a mono outline in
/// the theme's ink. Without this the folders (which come through `tab::folder_icon`, where the
/// flag is already set) came out blue while Trash, Filesystem and the rest came out as grey
/// outlines in the same list.
pub fn named(name: &'static str) -> widget::icon::Handle {
    icon::from_name(name).prefer_svg(true).size(16).handle()
}

/// A group header - bold, no icon, and indented less than the entries below it, so it reads
/// as the separator between groups (there are no divider lines in this sidebar).
pub fn header<M: 'static>(name: String, first: bool) -> Element<'static, M> {
    let cosmic_theme::Spacing {
        space_xxxs,
        space_xxs,
        space_xs,
        ..
    } = theme::spacing();
    widget::container(widget::text::heading(name))
        // The gap that separates the groups rides above the header; the first one sits flush
        // with the top of the sidebar.
        .padding([
            if first { 0 } else { space_xs },
            space_xxs,
            space_xxxs,
            space_xxs,
        ])
        .into()
}

/// Labels are ellipsized rather than wrapped - a long share name ("/ on johndoe...") must not
/// push a sidebar row onto a second line.
fn label(name: String) -> widget::Text<'static, cosmic::Theme, cosmic::Renderer> {
    use cosmic::iced::advanced::text::{Ellipsize, EllipsizeHeightLimit, Wrapping};
    widget::text(name)
        .wrapping(Wrapping::None)
        .ellipsize(Ellipsize::End(EllipsizeHeightLimit::Lines(1)))
        // Fill, so the paragraph is laid out against the sidebar width and actually reaches
        // the ellipsize limit instead of measuring its own (unbounded) intrinsic width.
        .width(Length::Fill)
}

/// One row (icon + label). `on_middle` opens the target in a background tab where the host
/// has tabs; pass `None` where it does not.
pub fn entry<M: Clone + 'static>(
    icon_handle: widget::icon::Handle,
    name: String,
    selected: bool,
    on_press: M,
    on_middle: Option<M>,
) -> Element<'static, M> {
    let cosmic_theme::Spacing {
        space_xxxs,
        space_xxs,
        space_xs,
        ..
    } = theme::spacing();
    let button = widget::button::custom(
        widget::row::with_children(vec![
            icon::icon(icon_handle).size(16).into(),
            label(name).into(),
        ])
        .spacing(space_xxs)
        .align_y(Alignment::Center),
    )
    .on_press(on_press)
    .padding([space_xxxs, space_xs])
    .class(if selected {
        selected_style()
    } else {
        theme::Button::ListItem([2.0; 4])
    })
    .width(Length::Fill);

    match on_middle {
        Some(msg) => MouseArea::new(button)
            .on_middle_press(move |_| msg.clone())
            .into(),
        None => button.into(),
    }
}

/// A drive row - label plus an optional thin disk-usage bar, all inside ONE clickable button
/// (the bar is part of the item; no separate "free" caption). `on_eject` adds the eject button
/// to the right of it.
pub fn drive<M: Clone + 'static>(
    icon_handle: widget::icon::Handle,
    name: String,
    selected: bool,
    fraction: Option<f32>,
    on_press: M,
    on_middle: Option<M>,
    on_eject: Option<M>,
) -> Element<'static, M> {
    let cosmic_theme::Spacing {
        space_xxxs,
        space_xxs,
        space_xs,
        ..
    } = theme::spacing();
    let row = widget::row::with_children(vec![
        icon::icon(icon_handle).size(16).into(),
        label(name).into(),
    ])
    .spacing(space_xxs)
    .align_y(Alignment::Center);
    let content: Element<'static, M> = match fraction {
        Some(f) => widget::column::with_children(vec![
            row.into(),
            // thinner than the default 4px girth via a fixed-height wrapper
            widget::container(widget::progress_bar::determinate_linear(f).width(Length::Fill))
                .width(Length::Fill)
                .height(Length::Fixed(3.0))
                .into(),
        ])
        .spacing(space_xxxs)
        .into(),
        None => row.into(),
    };
    let button = widget::button::custom(content)
        .on_press(on_press)
        .padding([space_xxxs, space_xs])
        .class(if selected {
            selected_style()
        } else {
            theme::Button::ListItem([2.0; 4])
        })
        .width(Length::Fill);
    let nav: Element<'static, M> = match on_middle {
        Some(msg) => MouseArea::new(button)
            .on_middle_press(move |_| msg.clone())
            .into(),
        None => button.into(),
    };

    match on_eject {
        Some(msg) => widget::row::with_children(vec![
            nav,
            widget::button::custom(widget::icon::from_name("media-eject-symbolic").size(16))
                .on_press(msg)
                .padding(space_xxs)
                .class(theme::Button::Icon)
                .into(),
        ])
        .align_y(Alignment::Center)
        .into(),
        None => nav,
    }
}

// WMDE: neutral selection appearance for sidebar items - gray bg (matches ListItem's selected
// background) but keeps text/icon neutral instead of the accent tint, so the selected entry
// reads as Win11-style (no blue icon). Every state maps to the same appearance, so hovering
// the current location does not flicker it back to the unselected look.
pub fn selected_style() -> theme::Button {
    theme::Button::Custom {
        active: Box::new(|_focused, theme| selected_appearance(theme)),
        disabled: Box::new(selected_appearance),
        hovered: Box::new(|_focused, theme| selected_appearance(theme)),
        pressed: Box::new(|_focused, theme| selected_appearance(theme)),
    }
}

fn selected_appearance(theme: &theme::Theme) -> widget::button::Style {
    let cosmic = theme.cosmic();
    let mut appearance = widget::button::Style::new();
    appearance.background =
        Some(cosmic::iced::Color::from(cosmic.primary(false).component.hover).into());
    appearance.text_color = Some(cosmic::iced::Color::from(cosmic.on_bg_color()));
    appearance.icon_color = Some(cosmic::iced::Color::from(cosmic.on_bg_color()));
    appearance.border_radius = [2.0_f32; 4].into();
    appearance
}
