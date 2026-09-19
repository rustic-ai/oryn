/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! `list` computed values.

use crate::{counter_style::CounterStyle, derives::*};
/// Native absolute tree identity is not CSS serialization. It is retained by
/// ordinary inheritance and by computed-to-specified animation conversion.
#[derive(Clone,Debug,Eq,MallocSizeOf,PartialEq,ToCss,ToResolvedValue,ToShmem,ToTyped)]
#[repr(C)]
#[typed(todo_derive_fields)]
pub struct ListStyleType(pub CounterStyle, #[css(skip)] pub usize);
impl ListStyleType {
    /// The initial disc marker style in the default tree scope.
    pub fn disc()->Self{Self(CounterStyle::disc(),0)}
    /// A suppressed marker in the default tree scope.
    pub fn none()->Self{Self(CounterStyle::None,0)}
    /// Whether the underlying counter style produces a bullet marker.
    pub fn is_bullet(&self)->bool{self.0.is_bullet()}
}
pub use crate::values::specified::list::Quotes;

impl Quotes {
    /// Initial value for `quotes`.
    #[inline]
    pub fn get_initial_value() -> Quotes {
        Quotes::Auto
    }
}
