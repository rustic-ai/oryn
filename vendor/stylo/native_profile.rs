/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */
//! Process-wide admission for the native layout consumer.
//!
//! CSSOM parsing can precede creation of a document. Initialize at both parser
//! and property-enumeration entry points so results cannot depend on which
//! thread first constructs a Stylist. Individually admitted longhands are in
//! the property tables; the blanket `layout.unimplemented` switch stays off.

/// Install the native consumer's supported preference profile once.
pub fn initialize() {
    #[cfg(feature = "servo")]
    {
        static INITIALIZE: std::sync::Once = std::sync::Once::new();
        INITIALIZE.call_once(|| {
            static_prefs::set_pref!("layout.grid.enabled", true);
            static_prefs::set_pref!("layout.columns.enabled", true);
            static_prefs::set_pref!("layout.writing-mode.enabled", true);
            static_prefs::set_pref!("layout.variable_fonts.enabled", true);
            static_prefs::set_pref!("layout.container-queries.enabled", true);
            static_prefs::set_pref!("layout.css.basic-shape-shape.enabled", true);
            static_prefs::set_pref!("layout.css.at-scope.enabled", true);
            static_prefs::set_pref!("layout.css.margin-rules.enabled", true);
            static_prefs::set_pref!("layout.css.font-tech.enabled", true);
            static_prefs::set_pref!("layout.css.content.alt-text.enabled", true);
        });
    }
}
