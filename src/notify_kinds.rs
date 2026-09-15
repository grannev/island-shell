//! The part of `notify` the icon preview needs (see examples/icons.rs).

/// Which vector icon stands for the sending app (see `icons`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppKind {
    Telegram,
    System,
    Firefox,
    Discord,
    Mail,
    Bell,
    Critical,
}
