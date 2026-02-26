// SPDX-FileCopyrightText: 2024 AerynOS Developers
// SPDX-License-Identifier: MPL-2.0

use std::io::stdout;

use crossterm::{style::Stylize, tty::IsTty};

macro_rules! impl_method {
    ($method:ident) => {
        fn $method(self) -> <Self as Stylize>::Styled {
            if stdout().is_tty() {
                <Self as Stylize>::$method(self)
            } else {
                self.stylize()
            }
        }
    };
}

/// Wrapper around `Stylized` which does nothing if not a TTY
pub trait Styled: Stylize {
    impl_method!(reset);
    impl_method!(bold);
    impl_method!(underlined);
    impl_method!(reverse);
    impl_method!(dim);
    impl_method!(italic);
    impl_method!(negative);
    impl_method!(slow_blink);
    impl_method!(rapid_blink);
    impl_method!(hidden);
    impl_method!(crossed_out);
    impl_method!(black);
    impl_method!(dark_grey);
    impl_method!(red);
    impl_method!(dark_red);
    impl_method!(green);
    impl_method!(dark_green);
    impl_method!(yellow);
    impl_method!(dark_yellow);
    impl_method!(blue);
    impl_method!(dark_blue);
    impl_method!(magenta);
    impl_method!(dark_magenta);
    impl_method!(cyan);
    impl_method!(dark_cyan);
    impl_method!(white);
    impl_method!(grey);
}

impl<T> Styled for T where T: Stylize {}
