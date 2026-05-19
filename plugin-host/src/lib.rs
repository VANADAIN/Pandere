mod bindings;
mod component;
mod dummy;
mod engine;
mod runtime_host;

pub use bindings::*;
pub use component::{LoadedPlugin, PluginHost};
pub use dummy::dummy_component_bytes;
pub use engine::component_engine;
pub use runtime_host::{HostState, LogEntry, RuntimeHost};

pub fn crate_name() -> &'static str {
    "pandere-plugin-host"
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bindings::pandere::messenger::types::Service;

    #[test]
    fn component_engine_enables_component_model() {
        let _engine = component_engine().expect("component engine should initialize");
    }

    #[test]
    fn host_state_round_trips_session_and_secret_values() {
        let mut state = HostState::default();
        state.store_session("telegram/session", "serialized-session");
        assert_eq!(
            state.load_session("telegram/session").as_deref(),
            Some("serialized-session")
        );

        let secret = state
            .store_secret("telegram-auth", "top-secret")
            .expect("secret should store");
        let loaded = state.load_secret(&secret).expect("secret should load");
        assert_eq!(loaded, "top-secret");
    }

    #[test]
    fn dummy_component_instantiates_via_bindgen() {
        let engine = component_engine().expect("component engine should initialize");
        let host = PluginHost::new(engine).expect("plugin host should initialize");
        let component_bytes = dummy_component_bytes().expect("dummy component should encode");
        let component = host
            .load_component_from_bytes(&component_bytes)
            .expect("dummy component should compile");

        let runtime = RuntimeHost::default();
        let mut plugin = host
            .instantiate(&component, runtime)
            .expect("component should instantiate");

        let metadata_error = plugin
            .metadata()
            .expect_err("dummy guest export should trap when called");
        let message = format!("{metadata_error:#}");
        assert!(
            message.contains("unreachable") || message.contains("wasm trap"),
            "unexpected trap error: {message}"
        );

        let _ = Service::Telegram;
    }
}
