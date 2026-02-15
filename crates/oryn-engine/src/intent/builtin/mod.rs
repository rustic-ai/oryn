use oryn_common::intent::registry::IntentRegistry;

pub mod accept_cookies;
pub mod dismiss_popups;
pub mod fill_form;
pub mod login;
pub mod logout;
pub mod scroll_to;
pub mod search;
pub mod submit_form;

/// Register all built-in intents into the registry.
pub fn register_all(registry: &mut IntentRegistry) {
    registry.register(login::definition());
    registry.register(search::definition());
    registry.register(accept_cookies::definition());
    registry.register(dismiss_popups::definition());
    registry.register(fill_form::definition());
    registry.register(submit_form::definition());
    registry.register(scroll_to::definition());
    registry.register(logout::definition());
}
