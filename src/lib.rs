// SPDX-License-Identifier: GPL-3.0-or-later
//! The wlRIX desktop background, as a library.
//!
//! The binary (`main.rs`) is a thin shell over this, so the config, the fitting rules, the
//! decoder, and the compositing can be exercised by tests without a compositor -- which is most of
//! the program. Only [`ui`] needs a Wayland connection.

pub mod config;
pub mod decode;
pub mod geometry;
pub mod pidfile;
pub mod render;
pub mod signals;
pub mod ui;
pub mod xdg;
