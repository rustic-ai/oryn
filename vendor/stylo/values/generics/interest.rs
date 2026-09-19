/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */
//! Interest invoker delay values.
use crate::derives::*;
/// The UA default delay or an explicitly nonnegative time.
#[derive(Animate,Clone,ComputeSquaredDistance,Copy,Debug,MallocSizeOf,PartialEq,SpecifiedValueInfo,ToAnimatedZero,ToComputedValue,ToCss,ToResolvedValue,ToShmem,ToTyped)]
#[repr(u8)]
pub enum InterestDelay<T>{
    /// Use the UA's start/end delay policy.
    #[animation(error)]
    Normal,
    /// An explicit nonnegative time.
    Time(T),
}

impl<T> crate::values::animated::ToAnimatedValue for InterestDelay<T>
where T:crate::values::animated::ToAnimatedValue+crate::values::generics::ClampToNonNegative{
    type AnimatedValue=InterestDelay<T::AnimatedValue>;
    fn to_animated_value(self,context:&crate::values::animated::Context)->Self::AnimatedValue{
        match self{Self::Normal=>InterestDelay::Normal,Self::Time(value)=>InterestDelay::Time(value.to_animated_value(context))}
    }
    fn from_animated_value(value:Self::AnimatedValue)->Self{
        match value{InterestDelay::Normal=>Self::Normal,InterestDelay::Time(value)=>Self::Time(T::from_animated_value(value).clamp_to_non_negative())}
    }
}
