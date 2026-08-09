//! Quiet Pepper Console tokens — paprika identity + ember action.
//!
//! Raw colors live only here (`docs/parity-plan.md` §3). Everywhere else
//! consumes these helpers or theme fields after [`apply_pimiento_brand`].

use gpui::{Hsla, rgb};
use gpui_component::{ActiveTheme as _, tag::Tag};
use pimiento_core::projection::RunPhase;

/// Identity paprika — mark / non-text brand ticks (not body copy).
pub(crate) fn identity_paprika() -> Hsla {
    rgb(0xc4_5c_26).into()
}

/// AA-safer action ember for primary fills / focus.
pub(crate) fn action_ember() -> Hsla {
    rgb(0xa8_3f_1a).into()
}

pub(crate) fn action_ember_hover() -> Hsla {
    rgb(0x0093_3716).into()
}

pub(crate) fn action_ember_fg() -> Hsla {
    rgb(0xff_ff_ff).into()
}

/// Status taxonomy (dot + label). Brand paprika is banned from this set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StatusKind {
    Working,
    Approval,
    AwaitingInput,
    Compacting,
    #[allow(dead_code)] // Reserved for completed transient operations, not a RunPhase.
    Done,
    Error,
    Idle,
}

impl StatusKind {
    pub(crate) fn from_run_phase(phase: &RunPhase) -> Self {
        match phase {
            RunPhase::Streaming => Self::Working,
            RunPhase::AwaitingResume => Self::AwaitingInput,
            RunPhase::Compacting | RunPhase::Retrying | RunPhase::Restarting => Self::Compacting,
            RunPhase::Dead => Self::Error,
            RunPhase::Idle => Self::Idle,
        }
    }

    pub(crate) fn from_phase_label(phase: &str) -> Self {
        let key = phase.trim_end_matches('…').trim_end_matches('.');
        match key {
            "stream" | "streaming" => Self::Working,
            "await" | "awaiting" => Self::AwaitingInput,
            "compact" | "compacting" | "retry" | "retrying" | "restart" | "restarting" => {
                Self::Compacting
            }
            "dead" => Self::Error,
            _ => Self::Idle,
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Working => "Working",
            Self::Approval => "Approval",
            Self::AwaitingInput => "Awaiting input",
            Self::Compacting => "Busy",
            Self::Done => "Done",
            Self::Error => "Error",
            Self::Idle => "Idle",
        }
    }

    /// Fill / fg / border for [`Tag::custom`].
    pub(crate) fn tag_colors(self) -> (Hsla, Hsla, Hsla) {
        match self {
            Self::Working => (
                rgb(0xe8_f1_fb).into(),
                rgb(0x1e_5a_8c).into(),
                rgb(0xb7_d4_ef).into(),
            ),
            Self::Approval | Self::Compacting => (
                rgb(0xfb_f0_dc).into(),
                rgb(0x8a_5a_12).into(),
                rgb(0xf0_d2_9a).into(),
            ),
            Self::AwaitingInput => (
                rgb(0xee_eb_f8).into(),
                rgb(0x45_3a_8c).into(),
                rgb(0xc9_c0_ea).into(),
            ),
            Self::Done => (
                rgb(0xe6_f6_ee).into(),
                rgb(0x1f_6b_4a).into(),
                rgb(0xb7_e0_cc).into(),
            ),
            Self::Error => (
                rgb(0xfb_e8_e6).into(),
                rgb(0x9b_2c_20).into(),
                rgb(0xf0_c0_b8).into(),
            ),
            Self::Idle => (
                rgb(0xf2_f2_f2).into(),
                rgb(0x55_55_55).into(),
                rgb(0xdd_dd_dd).into(),
            ),
        }
    }

    pub(crate) fn tag(self) -> Tag {
        let (bg, fg, border) = self.tag_colors();
        Tag::custom(bg, fg, border)
    }
}

/// Status pill Tag for a run phase (sentence-case label via caller `.child(...)`).
pub(crate) fn status_pill_for_phase(phase: &RunPhase) -> Tag {
    StatusKind::from_run_phase(phase).tag()
}

pub(crate) fn status_pill_for_label(phase: &str) -> Tag {
    StatusKind::from_phase_label(phase).tag()
}

/// Overlay paprika/ember onto the active gpui-component theme.
pub(crate) fn apply_pimiento_brand(cx: &mut gpui::App) {
    let mut theme = cx.theme().clone();
    let ember = action_ember();
    let hover = action_ember_hover();
    let fg = action_ember_fg();
    theme.colors.primary = ember;
    theme.colors.primary_hover = hover;
    theme.colors.primary_active = hover;
    theme.colors.primary_foreground = fg;
    theme.colors.button_primary = ember;
    theme.colors.button_primary_hover = hover;
    theme.colors.button_primary_active = hover;
    theme.colors.button_primary_foreground = fg;
    theme.colors.ring = ember;
    // Accent text / links lean paprika for identity without washing surfaces.
    theme.colors.accent = identity_paprika();
    theme.colors.accent_foreground = fg;
    // gpui-component renders controls from resolved tokens, so keep the
    // corresponding tokens synchronized with the legacy color fields.
    theme.tokens.primary = ember.into();
    theme.tokens.primary_hover = hover.into();
    theme.tokens.primary_active = hover.into();
    theme.tokens.primary_foreground = fg.into();
    theme.tokens.button_primary = ember.into();
    theme.tokens.button_primary_hover = hover.into();
    theme.tokens.button_primary_active = hover.into();
    theme.tokens.button_primary_foreground = fg.into();
    theme.tokens.ring = ember.into();
    theme.tokens.accent = identity_paprika().into();
    theme.tokens.accent_foreground = fg.into();
    cx.set_global(theme);
}
