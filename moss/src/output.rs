// SPDX-FileCopyrightText: Copyright © 2020-2026 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use std::sync::{Arc, OnceLock};

use crate::repository;

pub use self::tracing::TracingOutput;
pub use self::tui::TuiOutput;

mod tracing;
mod tui;

/// Default emitter used if [`install_emitter`] isn't called with a
/// custom [`Emit`] implementation.
pub type DefaultOutput = Chain<TracingOutput, TuiOutput>;

/// Global emitter either defaulted or installed via [`install_emitter`]
static EMITTER: OnceLock<Arc<dyn Emitter>> = OnceLock::new();

/// Install a global emitter that can be used with [`emit`]. If not called,
/// [`DefaultOutput`] is used.
///
/// This can only be called once. Future calls have no effect.
pub fn install_emitter(emitter: impl Emitter + 'static) {
    let _ = EMITTER.set(Arc::new(emitter));
}

/// Get access to the global emitter
pub fn emitter() -> &'static dyn Emitter {
    EMITTER.get_or_init(|| Arc::new(DefaultOutput::default())).as_ref()
}

/// Emit an event for output
#[macro_export]
macro_rules! emit {
    ($($tt:tt)*) => {
        $crate::output::Event::emit(($($tt)*), $crate::output::emitter());
    };
}

/// Defines how events are emitted to some output
pub trait Emitter: Send + Sync {
    fn emit(&self, _event: &InternalEvent) {}
}

/// An emittable event
pub trait Event: Sized {
    fn emit(self, _emitter: &dyn Emitter) {}
}

/// An internal `moss` library event
pub enum InternalEvent {
    RepositoryManager(repository::manager::OutputEvent),
}

pub trait EmitExt: Emitter + Sized {
    fn chain<U>(self, other: U) -> Chain<Self, U>
    where
        U: Emitter + Sized,
    {
        Chain { a: self, b: other }
    }
}

/// Do nothing with / suppress all output
#[derive(Debug, Clone, Copy)]
pub struct NoopOutput;

impl Emitter for NoopOutput {}

/// Chains multiple emitters together
#[derive(Clone, Default)]
pub struct Chain<A, B> {
    a: A,
    b: B,
}

impl<A, B> Emitter for Chain<A, B>
where
    A: Emitter,
    B: Emitter,
{
    fn emit(&self, event: &InternalEvent) {
        self.a.emit(event);
        self.b.emit(event);
    }
}
