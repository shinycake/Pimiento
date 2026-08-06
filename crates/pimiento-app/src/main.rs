#![forbid(unsafe_code)]
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! Pimiento M0 hello window.
//!
//! Themed `gpui-component` window: a primary button whose click bumps a counter
//! (visible in its label) and a light/dark toggle whose click flips the global
//! theme and refreshes its own label. UI-only smoke; no RPC yet.

use gpui::{ClickEvent, Context, Window, WindowOptions, div, prelude::*, px};
use gpui_component::{
    ActiveTheme, Root, Theme, ThemeMode,
    button::{Button, ButtonVariants as _},
    h_flex, v_flex,
};

#[derive(Debug)]
struct Hello {
    clicks: u32,
}

impl Hello {
    fn new() -> Self {
        Self { clicks: 0 }
    }

    fn on_primary(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.clicks = self.clicks.saturating_add(1);
        cx.notify();
    }

    /// Label shown on the primary button — pure function of `clicks`.
    fn primary_label(&self) -> String {
        if self.clicks == 0 {
            "Click me".to_owned()
        } else {
            format!("Clicked {}×", self.clicks)
        }
    }
}

/// Pure toggle: which mode should follow `current`.
fn next_theme_mode(current: ThemeMode) -> ThemeMode {
    if current.is_dark() {
        ThemeMode::Light
    } else {
        ThemeMode::Dark
    }
}

fn toggle_theme(_: &ClickEvent, window: &mut Window, cx: &mut gpui::App) {
    let next = next_theme_mode(cx.theme().mode);
    Theme::change(next, Some(window), cx);
}

impl Render for Hello {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let is_dark = cx.theme().mode.is_dark();
        let toggle_label = if is_dark {
            "Switch to Light"
        } else {
            "Switch to Dark"
        };
        let primary_label = self.primary_label();

        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap_4()
            .p_8()
            .text_color(cx.theme().foreground)
            .child(div().text_2xl().child("Pimiento"))
            .child(
                div()
                    .max_w(px(480.))
                    .text_center()
                    .text_color(cx.theme().muted_foreground)
                    .child(
                        "Native GPUI client for Oh My Pi. \
                         M0 skeleton — RPC and transcript arrive in M1+.",
                    ),
            )
            .child(
                h_flex()
                    .gap_3()
                    .child(
                        Button::new("primary")
                            .primary()
                            .label(primary_label)
                            .on_click(cx.listener(Self::on_primary)),
                    )
                    .child(
                        Button::new("theme-toggle")
                            .label(toggle_label)
                            .on_click(toggle_theme),
                    ),
            )
    }
}

fn main() {
    gpui_platform::application().run(|cx| {
        gpui_component::init(cx);

        cx.spawn(async move |cx| {
            cx.open_window(WindowOptions::default(), |window, cx| {
                let view = cx.new(|_| Hello::new());
                cx.new(|cx| Root::new(view, window, cx).bg(cx.theme().background))
            })
            .expect("open primary window");
        })
        .detach();
    });
}

#[cfg(test)]
mod tests {
    use super::{Hello, next_theme_mode};
    use gpui_component::ThemeMode;

    #[test]
    fn primary_label_initial_is_prompt() {
        let h = Hello::new();
        assert_eq!(h.primary_label(), "Click me");
    }

    #[test]
    fn primary_label_counts_clicks() {
        let mut h = Hello::new();
        h.clicks = 1;
        assert_eq!(h.primary_label(), "Clicked 1×");
        h.clicks = 42;
        assert_eq!(h.primary_label(), "Clicked 42×");
    }

    #[test]
    fn primary_label_saturates_at_u32_max() {
        let mut h = Hello::new();
        h.clicks = u32::MAX;
        // Saturation guarantees the counter never wraps to 0 = "Click me".
        assert_eq!(h.primary_label(), format!("Clicked {}×", u32::MAX));
        let before = h.clicks;
        h.clicks = h.clicks.saturating_add(1);
        assert_eq!(h.clicks, before);
    }

    #[test]
    fn next_theme_mode_flips_both_ways() {
        assert_eq!(next_theme_mode(ThemeMode::Light), ThemeMode::Dark);
        assert_eq!(next_theme_mode(ThemeMode::Dark), ThemeMode::Light);
        assert_eq!(
            next_theme_mode(next_theme_mode(ThemeMode::Light)),
            ThemeMode::Light,
        );
    }
}
